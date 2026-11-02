use std::{env, fs, path::PathBuf};

fn main() {
  // gpservice only accepts clients built for its exact release.
  let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../Cargo.toml");
  println!("cargo:rerun-if-changed={}", root.display());
  let manifest = fs::read_to_string(&root).expect("workspace Cargo.toml");
  let version = manifest
    .split("[workspace.package]")
    .nth(1)
    .and_then(|section| section.lines().find(|l| l.trim_start().starts_with("version")))
    .and_then(|line| line.split('"').nth(1))
    .expect("workspace.package.version");
  println!("cargo:rustc-env=GPAPI_VERSION={version}");
  tauri_build::build()
}
