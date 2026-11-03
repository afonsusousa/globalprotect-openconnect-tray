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

/// Profile ids name files under `sign-in/`, so only non-empty alphanumerics are allowed.
fn valid_id(id: &str) -> bool {
  !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
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
    if !valid_id(&self.id) {
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
    (profile.remember_sign_in && valid_id(&profile.id)).then(|| self.sign_in_dir.join(format!("{}.json", profile.id)))
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
    anyhow::ensure!(valid_id(id), "Invalid profile id");
    for ext in ["json", "clean"] {
      match fs::remove_file(self.sign_in_dir.join(format!("{id}.{ext}"))) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err.into()),
        _ => {}
      }
    }
    Ok(())
  }

  /// The "start clean" mark left by `forget_sign_in`, if set. The sign-in removes it
  /// only once it succeeds, so a failed attempt doesn't lose it.
  pub fn clean_marker(&self, id: &str) -> Option<PathBuf> {
    let path = self.sign_in_dir.join(format!("{id}.clean"));
    (valid_id(id) && path.is_file()).then_some(path)
  }

  pub fn has_saved_sign_in(&self, id: &str) -> bool {
    valid_id(id) && self.sign_in_dir.join(format!("{id}.json")).is_file()
  }

  fn read(&self) -> anyhow::Result<Vec<Profile>> {
    match fs::read_to_string(&self.path) {
      Ok(text) => serde_json::from_str(&text).with_context(|| format!("Cannot parse {}", self.path.display())),
      Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
      Err(err) => Err(err).with_context(|| format!("Cannot read {}", self.path.display())),
    }
  }

  pub fn load(&self) -> Vec<Profile> {
    self.read().unwrap_or_else(|err| {
      log::warn!("{err:#}");
      Vec::new()
    })
  }

  /// The profiles to update. An unreadable file is set aside (not overwritten) first,
  /// so a bad edit or a newer format never silently loses saved connections.
  fn load_for_update(&self) -> anyhow::Result<Vec<Profile>> {
    match self.read() {
      Ok(profiles) => Ok(profiles),
      Err(err) if self.path.is_file() => {
        let backup = self.path.with_extension(format!("json.unreadable-{}", new_id()));
        fs::rename(&self.path, &backup).with_context(|| format!("{err:#}; cannot move it aside"))?;
        log::warn!("{err:#}; moved it to {}", backup.display());
        Ok(Vec::new())
      }
      Err(err) => Err(err),
    }
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
    if profile.id.is_empty() {
      profile.id = new_id();
    }
    profile.validate()?;

    let mut profiles = self.load_for_update()?;
    match profiles.iter_mut().find(|p| p.id == profile.id) {
      Some(existing) => *existing = profile,
      None => profiles.push(profile),
    }
    self.write(&profiles)?;
    Ok(profiles)
  }

  pub fn delete(&self, id: &str) -> anyhow::Result<Vec<Profile>> {
    let mut profiles = self.load_for_update()?;
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
    let dir = std::env::temp_dir().join(format!("gp-tray-test-{}", new_id()));
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
    let marker = store.clean_marker("p1").unwrap();
    assert!(store.clean_marker("p1").is_some(), "peeking must not consume the mark");
    fs::remove_file(marker).unwrap();
    assert!(store.clean_marker("p1").is_none());
    assert!(store.clean_marker("").is_none());
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
    let dir = std::env::temp_dir().join(format!("gp-tray-test-{}", new_id()));
    let store = ProfileStore::new(dir.clone());
    let saved = store.save(Profile { id: String::new(), ..profile() }).unwrap();
    assert_eq!(saved.len(), 1);
    let id = saved[0].id.clone();
    assert!(!id.is_empty());
    assert_eq!(store.get(&id).unwrap().portal, "vpn.example.com");
    assert!(store.delete(&id).unwrap().is_empty());
    let _ = fs::remove_dir_all(dir);
  }

  #[test]
  fn unreadable_profiles_are_set_aside_not_overwritten() {
    let dir = std::env::temp_dir().join(format!("gp-tray-test-{}", new_id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("profiles.json"), "not json").unwrap();
    let store = ProfileStore::new(dir.clone());
    assert!(store.load().is_empty());
    store.save(Profile { id: String::new(), ..profile() }).unwrap();
    let kept: Vec<_> = fs::read_dir(&dir)
      .unwrap()
      .filter_map(|e| e.ok())
      .filter(|e| e.file_name().to_string_lossy().starts_with("profiles.json.unreadable-"))
      .collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(fs::read_to_string(kept[0].path()).unwrap(), "not json");
    let _ = fs::remove_dir_all(dir);
  }
}
