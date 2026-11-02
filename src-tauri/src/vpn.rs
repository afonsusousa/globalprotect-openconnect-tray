//! VPN control through `gpservice`, the same privileged backend the official GUI uses.
//!
//! 1. `gpservice --desktop-credentials` is started via pkexec. The packaged polkit
//!    policy (`com.yuezk.gpgui.service`, `allow_active=yes`) authorizes it without
//!    a password for the active desktop user, exactly as for the official GUI.
//! 2. The service writes a one-off session credential to our stdout pipe; we use it
//!    for the Noise-encrypted WebSocket handshake. Closing its stdin (i.e. quitting
//!    gptray) makes the service disconnect and exit.
//! 3. Sign-in runs unprivileged: `gpclient connect --cookie-only` does the portal,
//!    SAML/password and gateway login as the desktop user and prints the cookie.
//! 4. The cookie is handed to the service in a `Connect` request; it builds the tunnel.

use std::{
  collections::{HashMap, VecDeque},
  env,
  path::PathBuf,
  process::Stdio,
  sync::{Arc, Mutex},
  time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use gpapi::{
  gateway::Gateway,
  hip::HipSource,
  os_profile::{OsProfile, runtime_client_os},
  service::{
    event::WsEvent,
    request::{ConnectRequest, DisconnectRequest, WsRequest},
    transport::{
      ClientMessage, ClientPrelude, HIP_PROTOCOL_VERSION, MAX_WS_MESSAGE, NoiseInitiator, NoiseTransport, ServerMessage,
      ServerPrelude, ServiceResult, SessionCredential,
    },
    vpn_state::{ConnectInfo, VpnState},
  },
  session::{AuthenticatedGateway, ConnectionPlan, GatewayAuthentication, InternalSessionPolicy},
  utils::endpoint::ws_endpoint,
};
use serde::Serialize;
use tokio::{
  io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
  net::TcpStream,
  process::{ChildStdout, Command},
  sync::{mpsc, oneshot},
};
use tokio_tungstenite::{
  MaybeTlsStream, WebSocketStream, connect_async_with_config,
  tungstenite::{Message, protocol::WebSocketConfig},
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::profile::Profile;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const MAX_LOG_LINES: usize = 1000;
const MAX_CREDENTIAL_FRAME: usize = 4096;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Status {
  Disconnected,
  Connecting {
    profile_id: String,
    profile_name: String,
  },
  Connected {
    profile_id: Option<String>,
    profile_name: Option<String>,
    since: u64,
  },
  Disconnecting,
  Error {
    message: String,
  },
}

impl Status {
  pub fn label(&self) -> String {
    match self {
      Status::Disconnected => "Disconnected".into(),
      Status::Connecting { profile_name, .. } => format!("Connecting to {profile_name}…"),
      Status::Connected {
        profile_name: Some(name),
        ..
      } => format!("Connected to {name}"),
      Status::Connected { .. } => "Connected".into(),
      Status::Disconnecting => "Disconnecting…".into(),
      Status::Error { message } => format!("Error: {message}"),
    }
  }

  fn is_idle(&self) -> bool {
    matches!(self, Status::Disconnected | Status::Error { .. })
  }
}

#[derive(Serialize, Clone, Debug)]
pub struct LogLine {
  pub time: u64,
  pub level: String,
  pub message: String,
}

pub struct Hooks {
  pub on_status: Box<dyn Fn(&Status) + Send + Sync>,
  pub on_log: Box<dyn Fn(&LogLine) + Send + Sync>,
}

type Reply = oneshot::Sender<anyhow::Result<()>>;

#[derive(Default)]
struct Inner {
  status: Option<Status>,
  logs: VecDeque<LogLine>,
  last_error: Option<String>,
  /// The profile of the connection we started, for labelling service states.
  profile: Option<(String, String)>,
  /// Set while `gpclient --cookie-only` is signing in; sending cancels it.
  cancel_auth: Option<oneshot::Sender<()>>,
  /// Channel to the attached service session, once ready.
  service: Option<mpsc::Sender<(WsRequest, Reply)>>,
  service_starting: bool,
}

pub struct Vpn {
  inner: Mutex<Inner>,
  hooks: Hooks,
}

fn now() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or_default()
}

impl Vpn {
  pub fn new(hooks: Hooks) -> Self {
    Self {
      inner: Mutex::new(Inner::default()),
      hooks,
    }
  }

  pub fn status(&self) -> Status {
    self.inner.lock().unwrap().status.clone().unwrap_or(Status::Disconnected)
  }

  pub fn logs(&self) -> Vec<LogLine> {
    self.inner.lock().unwrap().logs.iter().cloned().collect()
  }

  pub fn clear_logs(&self) {
    self.inner.lock().unwrap().logs.clear();
  }

  fn set_status(&self, status: Status) {
    {
      let mut inner = self.inner.lock().unwrap();
      if inner.status.as_ref() == Some(&status) {
        return;
      }
      inner.status = Some(status.clone());
    }
    (self.hooks.on_status)(&status);
  }

  pub fn log(&self, level: &str, message: impl Into<String>) {
    let line = LogLine {
      time: now(),
      level: level.to_uppercase(),
      message: message.into(),
    };
    log::info!(target: "gptray", "{} {}", line.level, line.message);
    {
      let mut inner = self.inner.lock().unwrap();
      if line.level == "ERROR" && inner.last_error.is_none() {
        let first = line.message.lines().next().unwrap_or_default();
        let first = first.strip_prefix("Error: ").unwrap_or(first).trim();
        if !first.is_empty() {
          inner.last_error = Some(first.to_string());
        }
      }
      if inner.logs.len() >= MAX_LOG_LINES {
        inner.logs.pop_front();
      }
      inner.logs.push_back(line.clone());
    }
    (self.hooks.on_log)(&line);
  }

  /// gpclient emits JSON lines (`--log-format json`); gpservice and helpers emit text.
  fn log_raw(&self, raw: &str) {
    let raw = raw.trim_end();
    if raw.is_empty() {
      return;
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
      Ok(v) if v.get("message").is_some() => {
        let level = v["level"].as_str().unwrap_or("INFO");
        let message = v["message"].as_str().map(str::to_string).unwrap_or_else(|| v["message"].to_string());
        self.log(level, message);
      }
      _ => self.log(text_level(raw), raw),
    }
  }

  // ---- service session ------------------------------------------------------------

  /// Starts gpservice and keeps a session attached, unless one is already running.
  pub fn start_service(self: &Arc<Self>) {
    {
      let mut inner = self.inner.lock().unwrap();
      if inner.service.is_some() || inner.service_starting {
        return;
      }
      inner.service_starting = true;
    }
    let this = self.clone();
    tokio::spawn(async move {
      let result = this.clone().run_service().await;
      let was_busy = {
        let mut inner = this.inner.lock().unwrap();
        inner.service = None;
        inner.service_starting = false;
        !inner.status.as_ref().is_none_or(Status::is_idle)
      };
      if let Err(err) = result {
        this.log("ERROR", format!("VPN service stopped: {err:#}"));
        this.set_status(Status::Error {
          message: format!("{err:#}"),
        });
      } else if was_busy {
        this.set_status(Status::Disconnected);
      }
    });
  }

  async fn run_service(self: Arc<Self>) -> anyhow::Result<()> {
    let gpservice = common::binary_paths::gpservice();
    self.log("INFO", format!("Starting {} --desktop-credentials", gpservice.display()));
    let mut child = Command::new("pkexec")
      .arg("--user")
      .arg("root")
      .arg(&gpservice)
      .arg("--desktop-credentials")
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .context("Cannot start pkexec; is polkit installed?")?;

    // Held for the whole session: EOF tells the service to disconnect and exit.
    let _control = child.stdin.take().context("Service control pipe is unavailable")?;
    let mut output = child.stdout.take().context("Service credential pipe is unavailable")?;
    if let Some(stderr) = child.stderr.take() {
      spawn_reader(self.clone(), stderr);
    }

    let credential = tokio::select! {
      credential = read_credential(&mut output) => credential?,
      status = child.wait() => {
        let status = status?;
        match status.code() {
          Some(126 | 127) => bail!(
            "Not authorized to start gpservice. Is the GlobalProtect-openconnect polkit policy installed?"
          ),
          _ => bail!("gpservice exited during startup ({status}); see the log for details"),
        }
      }
    };
    ensure!(
      credential.product_version() == env!("GPAPI_VERSION"),
      "Installed gpservice {} does not match gptray's protocol version {}; rebuild gptray against the installed release",
      credential.product_version(),
      env!("GPAPI_VERSION")
    );

    let (tx, mut rx) = mpsc::channel::<(WsRequest, Reply)>(8);
    let mut tx = Some(tx);
    let mut failures = 0;
    loop {
      let attempt = tokio::select! {
        result = self.attach(&credential) => result,
        status = child.wait() => bail!("gpservice exited ({})", status?),
      };
      let (mut socket, mut noise) = match attempt {
        Ok(session) => {
          failures = 0;
          session
        }
        Err(err) => {
          failures += 1;
          ensure!(failures < 20, "Cannot attach to gpservice: {err:#}");
          tokio::time::sleep(Duration::from_millis(250)).await;
          continue;
        }
      };
      if let Some(tx) = tx.take() {
        let mut inner = self.inner.lock().unwrap();
        inner.service = Some(tx);
        inner.service_starting = false;
      }
      self.log("INFO", "Attached to gpservice");

      let mut pending: HashMap<Uuid, Reply> = HashMap::new();
      let result: anyhow::Result<()> = async {
        loop {
          tokio::select! {
            status = child.wait() => bail!("gpservice exited ({})", status?),
            command = rx.recv() => {
              let Some((request, reply)) = command else { return Ok(()) };
              let id = Uuid::new_v4();
              let message = ClientMessage::Request { id, request };
              match socket.send(Message::Binary(noise.encrypt(&message)?.into())).await {
                Ok(()) => { pending.insert(id, reply); }
                Err(err) => { let _ = reply.send(Err(err.into())); bail!("Service connection lost"); }
              }
            }
            frame = socket.next() => match frame.context("Service closed the connection")?? {
              Message::Binary(data) => match noise.decrypt::<ServerMessage>(&data)? {
                ServerMessage::Reply { id, result } => {
                  if let Some(reply) = pending.remove(&id) {
                    let _ = reply.send(match result {
                      ServiceResult::Rejected(rejection) => Err(anyhow::anyhow!("{}", rejection.message())),
                      _ => Ok(()),
                    });
                  }
                }
                ServerMessage::Event { event: WsEvent::VpnState(state), .. } => self.apply_service_state(state),
                ServerMessage::Event { event: WsEvent::VpnEnv(env), .. }
                | ServerMessage::Snapshot { snapshot: env, .. } => self.apply_service_state(env.vpn_state),
                ServerMessage::Event { .. } | ServerMessage::TunnelStatistics { .. } => {}
                ServerMessage::Attached { .. } => bail!("Service sent a duplicate attach response"),
              },
              Message::Ping(data) => socket.send(Message::Pong(data)).await?,
              Message::Close(_) => bail!("Service closed the connection"),
              _ => {}
            },
          }
        }
      }
      .await;
      for (_, reply) in pending.drain() {
        let _ = reply.send(Err(anyhow::anyhow!("Service connection lost")));
      }
      match result {
        Ok(()) => return Ok(()),
        Err(err) => self.log("WARN", format!("{err:#}; reattaching")),
      }
    }
  }

  async fn attach(&self, credential: &SessionCredential) -> anyhow::Result<(Socket, NoiseTransport)> {
    let endpoint = ws_endpoint().await.context("gpservice is not listening yet")?;
    let config = WebSocketConfig::default()
      .max_message_size(Some(MAX_WS_MESSAGE))
      .max_frame_size(Some(MAX_WS_MESSAGE));
    let (mut socket, _) = tokio::time::timeout(
      Duration::from_secs(5),
      connect_async_with_config(&endpoint, Some(config), false),
    )
    .await
    .context("Service connection timed out")??;
    let prelude = ClientPrelude {
      product_version: credential.product_version().to_owned(),
      hip_protocol_version: HIP_PROTOCOL_VERSION,
      service_instance_id: credential.service_instance_id(),
      session_id: credential.session_id(),
    };
    socket.send(Message::Binary(prelude.encode()?.into())).await?;
    match ServerPrelude::decode(&receive_binary(&mut socket).await?)? {
      ServerPrelude::Ready => {}
      ServerPrelude::Rejected(reason) => bail!("Service rejected the session: {reason:?}"),
    }
    let mut initiator = NoiseInitiator::new(
      credential.product_version(),
      credential.service_instance_id(),
      credential.session_id(),
      credential.secret(),
    )?;
    socket.send(Message::Binary(initiator.write_first()?.into())).await?;
    let mut noise = initiator.read_response(&receive_binary(&mut socket).await?)?;
    let attach = ClientMessage::Attach {
      last_event_revision: None,
    };
    socket.send(Message::Binary(noise.encrypt(&attach)?.into())).await?;
    match noise.decrypt::<ServerMessage>(&receive_binary(&mut socket).await?)? {
      ServerMessage::Attached { snapshot, .. } => self.apply_service_state(snapshot.vpn_state),
      _ => bail!("Service did not attach the session"),
    }
    Ok((socket, noise))
  }

  fn apply_service_state(&self, state: VpnState) {
    let (signing_in, profile) = {
      let inner = self.inner.lock().unwrap();
      (inner.cancel_auth.is_some(), inner.profile.clone())
    };
    let (profile_id, profile_name) = match profile {
      Some((id, name)) => (Some(id), Some(name)),
      None => (None, None),
    };
    let status = match state {
      // While we are signing in, the service is idle; keep showing "Connecting".
      VpnState::Disconnected if signing_in => return,
      VpnState::Disconnected => Status::Disconnected,
      VpnState::Connecting(info) => Status::Connecting {
        profile_id: profile_id.unwrap_or_default(),
        profile_name: profile_name.unwrap_or_else(|| info.portal().to_string()),
      },
      VpnState::Connected(connected) => {
        let name = profile_name.or_else(|| Some(connected.info().portal().to_string()));
        Status::Connected {
          profile_id,
          profile_name: name,
          since: u64::from(connected.connected_at()),
        }
      }
      VpnState::Disconnecting => Status::Disconnecting,
      VpnState::Failed(message) => Status::Error { message },
      VpnState::ReconnectRequired(_) => Status::Error {
        message: "The VPN session expired. Connect again to sign in.".into(),
      },
    };
    self.set_status(status);
  }

  async fn request(&self, request: WsRequest) -> anyhow::Result<()> {
    let sender = self.inner.lock().unwrap().service.clone();
    let sender = match sender {
      Some(sender) => sender,
      None => self.wait_for_service().await?,
    };
    let (reply_tx, reply_rx) = oneshot::channel();
    sender
      .send((request, reply_tx))
      .await
      .map_err(|_| anyhow::anyhow!("VPN service is not running"))?;
    reply_rx.await.map_err(|_| anyhow::anyhow!("VPN service did not reply"))?
  }

  async fn wait_for_service(&self) -> anyhow::Result<mpsc::Sender<(WsRequest, Reply)>> {
    for _ in 0..80 {
      if let Some(sender) = self.inner.lock().unwrap().service.clone() {
        return Ok(sender);
      }
      tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("VPN service is not available; check the log for details")
  }

  // ---- connect / disconnect -----------------------------------------------------

  pub async fn connect(
    self: Arc<Self>,
    profile: Profile,
    password: Option<String>,
    sign_in_cache: Option<PathBuf>,
  ) -> anyhow::Result<()> {
    if !self.status().is_idle() {
      bail!("A connection is already in progress");
    }
    profile.validate()?;
    let password = password.filter(|p| !p.is_empty()).map(Zeroizing::new);
    if profile.needs_password() && password.is_none() {
      bail!("A password is required for this connection");
    }
    let gpclient = find_gpclient()?;

    self.start_service();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    {
      let mut inner = self.inner.lock().unwrap();
      inner.last_error = None;
      inner.cancel_auth = Some(cancel_tx);
      inner.profile = Some((profile.id.clone(), profile.name.clone()));
    }
    self.set_status(Status::Connecting {
      profile_id: profile.id.clone(),
      profile_name: profile.name.clone(),
    });

    let this = self.clone();
    tokio::spawn(async move {
      let result = this
        .sign_in_and_connect(&gpclient, &profile, password, sign_in_cache, cancel_rx)
        .await;
      let cancelled = this.inner.lock().unwrap().cancel_auth.take().is_none();
      match result {
        Ok(()) => {}
        Err(_) if cancelled => {
          this.log("INFO", "Sign-in cancelled");
          this.set_status(Status::Disconnected);
        }
        Err(err) => {
          let message = this
            .inner
            .lock()
            .unwrap()
            .last_error
            .take()
            .unwrap_or_else(|| format!("{err:#}"));
          this.log("ERROR", format!("{err:#}"));
          this.set_status(Status::Error { message });
        }
      }
    });
    Ok(())
  }

  async fn sign_in_and_connect(
    self: &Arc<Self>,
    gpclient: &PathBuf,
    profile: &Profile,
    password: Option<Zeroizing<String>>,
    sign_in_cache: Option<PathBuf>,
    cancel: oneshot::Receiver<()>,
  ) -> anyhow::Result<()> {
    if let Some(dir) = sign_in_cache.as_deref().and_then(|p| p.parent()) {
      create_private_dir(dir)?;
    }
    let args = profile.sign_in_args(sign_in_cache.as_deref());
    self.log("INFO", format!("Signing in: {} {}", gpclient.display(), args.join(" ")));
    let mut child = Command::new(gpclient)
      .args(&args)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .kill_on_drop(true)
      .spawn()
      .with_context(|| format!("Cannot run {}", gpclient.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
      if let Some(password) = &password {
        let line = Zeroizing::new(format!("{}\n", password.as_str()));
        stdin.write_all(line.as_bytes()).await.context("Cannot send the password")?;
      }
      // Closing stdin makes unexpected prompts fail instead of hanging.
      drop(stdin);
    }
    drop(password);
    if let Some(stderr) = child.stderr.take() {
      spawn_reader(self.clone(), stderr);
    }
    let stdout = child.stdout.take().context("gpclient output is unavailable")?;

    let (output, status) = tokio::select! {
      result = async { tokio::join!(read_cookie_output(stdout), child.wait()) } => result,
      _ = cancel => {
        let _ = child.kill().await;
        bail!("cancelled");
      }
    };
    let status = status?;
    let output = output?;
    for line in &output.other {
      self.log_raw(line);
    }
    if !status.success() {
      bail!("Sign-in failed (gpclient exited with {status})");
    }
    let (cookie, host) = output.into_parts().context("gpclient did not print a gateway cookie")?;
    self.log("INFO", format!("Signed in; connecting to gateway {host}"));

    // From here the service owns progress reporting.
    {
      let mut inner = self.inner.lock().unwrap();
      if inner.cancel_auth.take().is_none() {
        bail!("cancelled");
      }
    }
    let request = build_connect_request(profile, cookie, host)?;
    self.request(WsRequest::Connect(Box::new(request))).await
  }

  /// Cancels a sign-in in progress, or asks the service to disconnect.
  pub async fn disconnect(self: Arc<Self>) -> anyhow::Result<()> {
    if let Some(cancel) = self.inner.lock().unwrap().cancel_auth.take() {
      let _ = cancel.send(());
      return Ok(());
    }
    if self.inner.lock().unwrap().service.is_none() {
      self.set_status(Status::Disconnected);
      return Ok(());
    }
    self.set_status(Status::Disconnecting);
    self.request(WsRequest::Disconnect(DisconnectRequest)).await
  }
}

fn build_connect_request(profile: &Profile, cookie: String, host: String) -> anyhow::Result<ConnectRequest> {
  let gateway = Gateway::new(host.clone(), host);
  let info = ConnectInfo::new(profile.portal.clone(), gateway.clone(), vec![gateway.clone()]);
  let plan = ConnectionPlan::new(
    vec![AuthenticatedGateway {
      gateway,
      authentication: GatewayAuthentication::new(cookie, "tunnel".into())?,
      binding: None,
    }],
    vec![],
    InternalSessionPolicy::default(),
  )?;
  // Same OS profile gpclient used for gateway login, so the cookie stays valid.
  let os_profile = OsProfile::builder(runtime_client_os()).build();
  Ok(
    ConnectRequest::new(info, plan)
      .with_os_profile(&os_profile)
      .with_ignore_tls_errors(profile.ignore_tls_errors)
      .with_hip_source(if profile.hip {
        HipSource::Generated
      } else {
        HipSource::Disabled
      }),
  )
}

#[derive(Default)]
struct CookieOutput {
  cookie: Option<String>,
  host: Option<String>,
  other: Vec<String>,
}

impl CookieOutput {
  fn into_parts(self) -> Option<(String, String)> {
    Some((self.cookie?, self.host?))
  }
}

fn quoted<'a>(line: &'a str, key: &str) -> Option<&'a str> {
  line.strip_prefix(key)?.strip_prefix("='")?.strip_suffix('\'')
}

/// Parses `COOKIE='…'` / `HOST='…'` from `gpclient --cookie-only`. The cookie is never logged.
async fn read_cookie_output(stdout: ChildStdout) -> anyhow::Result<CookieOutput> {
  let mut out = CookieOutput::default();
  let mut lines = BufReader::new(stdout).lines();
  while let Some(line) = lines.next_line().await? {
    parse_cookie_line(&mut out, line);
  }
  Ok(out)
}

fn parse_cookie_line(out: &mut CookieOutput, line: String) {
  let trimmed = line.trim();
  if let Some(cookie) = quoted(trimmed, "COOKIE") {
    out.cookie = Some(cookie.to_string());
  } else if let Some(host) = quoted(trimmed, "HOST") {
    out.host = Some(host.to_string());
  } else if !trimmed.is_empty() {
    out.other.push(line);
  }
}

async fn read_credential(reader: &mut ChildStdout) -> anyhow::Result<SessionCredential> {
  let mut header = [0u8; 2];
  reader
    .read_exact(&mut header)
    .await
    .context("gpservice closed before issuing a session credential")?;
  let length = usize::from(u16::from_be_bytes(header));
  ensure!(
    length != 0 && length + 2 <= MAX_CREDENTIAL_FRAME,
    "Invalid service credential length"
  );
  let mut frame = Zeroizing::new(vec![0; length + 2]);
  frame[..2].copy_from_slice(&header);
  reader.read_exact(&mut frame[2..]).await?;
  Ok(SessionCredential::decode_frame(&frame)?)
}

async fn receive_binary(socket: &mut Socket) -> anyhow::Result<Vec<u8>> {
  match tokio::time::timeout(Duration::from_secs(5), socket.next())
    .await
    .context("Service handshake timed out")?
  {
    Some(Ok(Message::Binary(data))) => Ok(data.to_vec()),
    Some(Ok(_)) => bail!("Service sent an unexpected handshake frame"),
    Some(Err(err)) => Err(err.into()),
    None => bail!("Service closed during handshake"),
  }
}

fn text_level(line: &str) -> &'static str {
  for level in ["ERROR", "WARN", "DEBUG", "TRACE"] {
    if line.contains(&format!(" {level} ")) || line.contains(&format!("[{level}")) {
      return level;
    }
  }
  "INFO"
}

fn spawn_reader<R: AsyncRead + Unpin + Send + 'static>(vpn: Arc<Vpn>, stream: R) {
  tokio::spawn(async move {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
      vpn.log_raw(&line);
    }
  });
}

fn create_private_dir(dir: &std::path::Path) -> anyhow::Result<()> {
  use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
  std::fs::DirBuilder::new()
    .recursive(true)
    .mode(0o700)
    .create(dir)
    .with_context(|| format!("Cannot create {}", dir.display()))?;
  std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
  Ok(())
}

fn find_gpclient() -> anyhow::Result<PathBuf> {
  if let Some(path) = env::var_os("GPTRAY_GPCLIENT").map(PathBuf::from) {
    ensure!(path.is_file(), "GPTRAY_GPCLIENT does not point to a file");
    return Ok(path);
  }
  let path = common::binary_paths::gpclient();
  ensure!(
    path.is_file(),
    "Cannot find gpclient at {}; is GlobalProtect-openconnect installed?",
    path.display()
  );
  Ok(path)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn vpn() -> Vpn {
    Vpn::new(Hooks {
      on_status: Box::new(|_| {}),
      on_log: Box::new(|_| {}),
    })
  }

  #[test]
  fn cookie_output_is_parsed_and_other_lines_kept() {
    let mut out = CookieOutput::default();
    for line in [
      "COOKIE='authcookie=abc&user=alice'",
      "something else",
      "HOST='gw.example.com'",
    ] {
      parse_cookie_line(&mut out, line.to_string());
    }
    assert_eq!(out.other, ["something else"]);
    assert_eq!(
      out.into_parts(),
      Some(("authcookie=abc&user=alice".to_string(), "gw.example.com".to_string()))
    );
  }

  #[test]
  fn connect_request_carries_gateway_cookie() {
    let profile = Profile {
      portal: "vpn.example.com".into(),
      name: "Work".into(),
      ..Default::default()
    };
    let request = build_connect_request(&profile, "authcookie=abc&user=alice".into(), "gw.example.com".into()).unwrap();
    assert_eq!(request.plan().members()[0].gateway.server(), "gw.example.com");
    assert!(build_connect_request(&profile, "nonsense".into(), "gw".into()).is_err());
  }

  #[test]
  fn json_logs_are_parsed_and_first_error_is_kept() {
    let vpn = vpn();
    vpn.log_raw(r#"{"timestamp":"t","level":"INFO","target":"gpclient","message":"hello"}"#);
    vpn.log_raw(r#"{"level":"ERROR","message":"Error: portal unreachable\n\nCaused by: x"}"#);
    vpn.log_raw(r#"{"level":"ERROR","message":"hint"}"#);
    vpn.log_raw("[2026-11-02T20:00:00Z WARN  gpservice] plain text");
    let logs = vpn.logs();
    assert_eq!(logs.len(), 4);
    assert_eq!(logs[0].message, "hello");
    assert_eq!(logs[3].level, "WARN");
    assert_eq!(vpn.inner.lock().unwrap().last_error.as_deref(), Some("portal unreachable"));
  }

  #[test]
  fn status_serializes_for_the_frontend() {
    let json = serde_json::to_value(Status::Connecting {
      profile_id: "a".into(),
      profile_name: "Work".into(),
    })
    .unwrap();
    assert_eq!(json, serde_json::json!({"state": "connecting", "profileId": "a", "profileName": "Work"}));
  }

  #[test]
  fn service_states_map_to_status() {
    let vpn = vpn();
    vpn.inner.lock().unwrap().profile = Some(("p1".into(), "Work".into()));
    vpn.apply_service_state(VpnState::Failed("boom".into()));
    assert_eq!(vpn.status(), Status::Error { message: "boom".into() });
    vpn.apply_service_state(VpnState::Disconnecting);
    assert_eq!(vpn.status(), Status::Disconnecting);

    // While signing in, an idle service must not reset our "Connecting" state.
    let (tx, _rx) = oneshot::channel();
    vpn.inner.lock().unwrap().cancel_auth = Some(tx);
    vpn.set_status(Status::Connecting {
      profile_id: "p1".into(),
      profile_name: "Work".into(),
    });
    vpn.apply_service_state(VpnState::Disconnected);
    assert!(matches!(vpn.status(), Status::Connecting { .. }));
  }
}
