//! Saved connection profiles and how they map onto `gpclient` arguments.

use std::{
  fs,
  path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
  /// gpclient's built-in webview sign-in (gpauth).
  #[default]
  Embedded,
  /// SAML sign-in in an external browser (`--browser`).
  Browser,
  /// Username and password, passed on stdin (`--passwd-on-stdin`).
  Password,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
  pub id: String,
  pub name: String,
  pub portal: String,
  /// Empty means "try gateways automatically in priority order".
  #[serde(default)]
  pub gateway: String,
  #[serde(default)]
  pub as_gateway: bool,
  #[serde(default)]
  pub auth: AuthMethod,
  /// Browser for `AuthMethod::Browser`; empty means auto-select.
  #[serde(default)]
  pub browser: String,
  #[serde(default)]
  pub username: String,
  #[serde(default)]
  pub hip: bool,
  /// Reuse the portal's sign-in cookie so SAML/browser login isn't needed every time.
  /// A new key (not the old `cookieCache`) so existing profiles start with it enabled.
  #[serde(default = "enabled")]
  pub remember_sign_in: bool,
  /// On by default (user preference); can be turned off per connection in Settings.
  #[serde(default = "enabled")]
  pub ignore_tls_errors: bool,
}

fn enabled() -> bool {
  true
}

/// Values are passed to a privileged process, so refuse anything that could be read as an option.
fn check_value(field: &str, value: &str, required: bool) -> anyhow::Result<()> {
  if required && value.trim().is_empty() {
    bail!("{field} is required");
  }
  if value.starts_with('-') {
    bail!("{field} must not start with '-'");
  }
  if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
    bail!("{field} must not contain spaces or control characters");
  }
  Ok(())
}

impl Profile {
  pub fn validate(&self) -> anyhow::Result<()> {
    if self.name.trim().is_empty() {
      bail!("Name is required");
    }
    // The id names this profile's sign-in cache file.
    if !self.id.chars().all(|c| c.is_ascii_alphanumeric()) {
      bail!("Invalid profile id");
    }
    check_value("Portal", &self.portal, true)?;
    check_value("Gateway", &self.gateway, false)?;
    check_value("Browser", &self.browser, false)?;
    check_value("Username", &self.username, self.auth == AuthMethod::Password)?;
    Ok(())
  }

  pub fn needs_password(&self) -> bool {
    self.auth == AuthMethod::Password
  }

  /// Arguments for an unprivileged `gpclient` sign-in that prints the gateway cookie.
  /// HIP is not passed here: gpservice submits HIP reports once the tunnel is up.
  /// `sign_in_cache` is this profile's portal cookie file, when sign-in should be remembered.
  /// `clean` drops the built-in sign-in window's remembered session (after "Delete credentials").
  pub fn sign_in_args(&self, sign_in_cache: Option<&Path>, clean: bool) -> Vec<String> {
    let mut args: Vec<String> = vec!["--log-format".into(), "json".into()];
    if self.ignore_tls_errors {
      args.push("--ignore-tls-errors".into());
    }
    args.extend(["connect".into(), "--cookie-only".into()]);

    if self.as_gateway {
      args.push("--as-gateway".into());
    } else if self.gateway.is_empty() {
      // Without this gpclient would prompt on a terminal we don't have.
      args.push("--auto-gateway".into());
    } else {
      args.extend(["--gateway".into(), self.gateway.clone()]);
    }

    match self.auth {
      AuthMethod::Embedded if clean => args.push("--clean".into()),
      AuthMethod::Embedded => {}
      AuthMethod::Browser => {
        let browser = if self.browser.is_empty() { "auto" } else { &self.browser };
        args.push(format!("--browser={browser}"));
      }
      AuthMethod::Password => {
        args.extend(["--user".into(), self.username.clone(), "--passwd-on-stdin".into()]);
      }
    }

    if let Some(path) = sign_in_cache {
      // One file per profile: gpclient's default cache holds a single portal.
      args.push(format!("--cookie-cache={}", path.display()));
    }

    args.extend(["--".into(), self.portal.clone()]);
    args
  }
}

pub struct ProfileStore {
  path: PathBuf,
  sign_in_dir: PathBuf,
}

impl ProfileStore {
  pub fn new(dir: PathBuf) -> Self {
    Self {
      path: dir.join("profiles.json"),
      sign_in_dir: dir.join("sign-in"),
    }
  }

  /// Where gpclient caches this profile's portal cookie (created by gpclient, mode 0600).
  pub fn sign_in_cache(&self, profile: &Profile) -> Option<PathBuf> {
    let valid = !profile.id.is_empty() && profile.id.chars().all(|c| c.is_ascii_alphanumeric());
    (profile.remember_sign_in && valid).then(|| self.sign_in_dir.join(format!("{}.json", profile.id)))
  }

  /// Deletes the saved sign-in so the next connect asks to log in again, and marks the
  /// profile so that sign-in also discards the built-in window's remembered session.
  pub fn forget_sign_in(&self, id: &str) -> anyhow::Result<()> {
    self.remove_sign_in(id)?;
    fs::create_dir_all(&self.sign_in_dir)?;
    fs::write(self.sign_in_dir.join(format!("{id}.clean")), b"")?;
    Ok(())
  }

  fn remove_sign_in(&self, id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(id.chars().all(|c| c.is_ascii_alphanumeric()), "Invalid profile id");
    for ext in ["json", "clean"] {
      match fs::remove_file(self.sign_in_dir.join(format!("{id}.{ext}"))) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err.into()),
        _ => {}
      }
    }
    Ok(())
  }

  /// Consumes the "start clean" mark left by `forget_sign_in`.
  pub fn take_clean(&self, id: &str) -> bool {
    id.chars().all(|c| c.is_ascii_alphanumeric()) && fs::remove_file(self.sign_in_dir.join(format!("{id}.clean"))).is_ok()
  }

  pub fn has_saved_sign_in(&self, id: &str) -> bool {
    id.chars().all(|c| c.is_ascii_alphanumeric()) && self.sign_in_dir.join(format!("{id}.json")).is_file()
  }

  pub fn load(&self) -> Vec<Profile> {
    fs::read_to_string(&self.path)
      .ok()
      .and_then(|s| serde_json::from_str(&s).ok())
      .unwrap_or_default()
  }

  fn write(&self, profiles: &[Profile]) -> anyhow::Result<()> {
    if let Some(dir) = self.path.parent() {
      fs::create_dir_all(dir).with_context(|| format!("Cannot create {}", dir.display()))?;
    }
    let tmp = self.path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(profiles)?)?;
    fs::rename(&tmp, &self.path)?;
    Ok(())
  }

  pub fn get(&self, id: &str) -> Option<Profile> {
    self.load().into_iter().find(|p| p.id == id)
  }

  pub fn save(&self, mut profile: Profile) -> anyhow::Result<Vec<Profile>> {
    profile.name = profile.name.trim().to_string();
    profile.portal = profile.portal.trim().to_string();
    profile.gateway = profile.gateway.trim().to_string();
    profile.browser = profile.browser.trim().to_string();
    profile.username = profile.username.trim().to_string();
    profile.validate()?;

    let mut profiles = self.load();
    if profile.id.is_empty() {
      profile.id = new_id();
    }
    match profiles.iter_mut().find(|p| p.id == profile.id) {
      Some(existing) => *existing = profile,
      None => profiles.push(profile),
    }
    self.write(&profiles)?;
    Ok(profiles)
  }

  pub fn delete(&self, id: &str) -> anyhow::Result<Vec<Profile>> {
    let mut profiles = self.load();
    profiles.retain(|p| p.id != id);
    self.write(&profiles)?;
    let _ = self.remove_sign_in(id);
    Ok(profiles)
  }
}

fn new_id() -> String {
  let nanos = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_nanos())
    .unwrap_or_default();
  format!("p{nanos:x}")
}

#[cfg(test)]
mod tests {
  use super::*;

  fn profile() -> Profile {
    Profile {
      id: "x".into(),
      name: "Work".into(),
      portal: "vpn.example.com".into(),
      ..Default::default()
    }
  }

  #[test]
  fn default_profile_uses_auto_gateway_and_embedded_auth() {
    assert_eq!(
      profile().sign_in_args(None, false),
      ["--log-format", "json", "connect", "--cookie-only", "--auto-gateway", "--", "vpn.example.com"]
    );
  }

  #[test]
  fn password_profile_reads_password_from_stdin() {
    let p = Profile {
      auth: AuthMethod::Password,
      username: "alice".into(),
      gateway: "gw1".into(),
      ignore_tls_errors: true,
      ..profile()
    };
    assert_eq!(
      p.sign_in_args(None, false),
      [
        "--log-format",
        "json",
        "--ignore-tls-errors",
        "connect",
        "--cookie-only",
        "--gateway",
        "gw1",
        "--user",
        "alice",
        "--passwd-on-stdin",
        "--",
        "vpn.example.com"
      ]
    );
  }

  #[test]
  fn browser_profile_defaults_to_auto() {
    let p = Profile {
      auth: AuthMethod::Browser,
      hip: true,
      as_gateway: true,
      ..profile()
    };
    assert_eq!(
      p.sign_in_args(None, false),
      [
        "--log-format",
        "json",
        "connect",
        "--cookie-only",
        "--as-gateway",
        "--browser=auto",
        "--",
        "vpn.example.com"
      ]
    );
  }

  #[test]
  fn sign_in_cache_is_per_profile_and_on_by_default() {
    let dir = std::env::temp_dir().join(format!("gptray-test-{}", new_id()));
    let store = ProfileStore::new(dir.clone());
    // Profiles saved before this option existed (or with the old `cookieCache` key) default to on.
    let old: Profile = serde_json::from_str(r#"{"id":"p1","name":"W","portal":"vpn.example.com","cookieCache":false}"#).unwrap();
    assert!(old.remember_sign_in);
    let path = store.sign_in_cache(&old).unwrap();
    assert!(path.ends_with("sign-in/p1.json"));
    let args = old.sign_in_args(Some(&path), false);
    assert!(args.contains(&format!("--cookie-cache={}", path.display())));

    assert!(store.sign_in_cache(&Profile { remember_sign_in: false, ..old.clone() }).is_none());
    assert!(store.sign_in_cache(&Profile { id: "../x".into(), ..old.clone() }).is_none());
    assert!(store.forget_sign_in("../x").is_err());

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{}").unwrap();
    assert!(store.has_saved_sign_in("p1"));
    store.forget_sign_in("p1").unwrap();
    assert!(!store.has_saved_sign_in("p1"));
    // After deleting credentials, the next embedded sign-in starts clean, once.
    assert!(old.sign_in_args(None, true).contains(&"--clean".to_string()));
    assert!(store.take_clean("p1"));
    assert!(!store.take_clean("p1"));
    let _ = fs::remove_dir_all(dir);
  }

  #[test]
  fn rejects_option_like_values() {
    assert!(Profile { portal: "--script=/x".into(), ..profile() }.validate().is_err());
    assert!(Profile { gateway: "-g".into(), ..profile() }.validate().is_err());
    assert!(Profile { portal: "a b".into(), ..profile() }.validate().is_err());
    assert!(Profile { auth: AuthMethod::Password, ..profile() }.validate().is_err());
    assert!(Profile { id: "../evil".into(), ..profile() }.validate().is_err());
    assert!(profile().validate().is_ok());
  }

  #[test]
  fn store_round_trips() {
    let dir = std::env::temp_dir().join(format!("gptray-test-{}", new_id()));
    let store = ProfileStore::new(dir.clone());
    let saved = store.save(Profile { id: String::new(), ..profile() }).unwrap();
    assert_eq!(saved.len(), 1);
    let id = saved[0].id.clone();
    assert!(!id.is_empty());
    assert_eq!(store.get(&id).unwrap().portal, "vpn.example.com");
    assert!(store.delete(&id).unwrap().is_empty());
    let _ = fs::remove_dir_all(dir);
  }
}
