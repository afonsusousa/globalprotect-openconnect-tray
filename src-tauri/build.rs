use std::{env, fs, path::PathBuf};

/// The globalprotect-openconnect release this tray targets, taken from the `tag = "vX.Y.Z"`
/// that pins the gpapi git dependency in Cargo.toml.
fn gp_version(manifest: &str) -> Option<String> {
  let line = manifest.lines().find(|l| l.trim_start().starts_with("gpapi"))?;
  let tag = line.split("tag").nth(1)?.split('"').nth(1)?;
  Some(tag.trim_start_matches('v').to_string())
}

fn main() {
  let manifest_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("Cargo.toml");
  println!("cargo:rerun-if-changed={}", manifest_path.display());
  let manifest = fs::read_to_string(&manifest_path).expect("Cargo.toml");
  let version = gp_version(&manifest).expect("gpapi must be a git dependency pinned with tag = \"vX.Y.Z\"");
  // gpservice only accepts clients built for its exact release.
  println!("cargo:rustc-env=GPAPI_VERSION={version}");
  tauri_build::build()
}
