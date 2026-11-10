//! Packaging a Rust service, at build time, behind the `build` feature. Its
//! build script calls `package()`, which reads the crate's `service.toml` and
//! writes what the crate embeds: its name, its version, its `sql` binding and
//! its timeout. How it sets up its storage is its own `init`'s concern.
//!
//! The version is the hash of every file in the crate (see `manifest`).

use std::path::PathBuf;

use crate::manifest::{Manifest, version};

// The Rust a package embeds: its name, version, sql binding and timeout.
pub fn generate(manifest: &Manifest, version: &str) -> String {
  format!(
    "pub const NAME: &str = {:?};\npub const VERSION: &str = {version:?};\npub const BINDING: Option<&str> = {:?};\npub const TIMEOUT: Option<u64> = {:?};\n",
    manifest.name,
    manifest.binding(),
    manifest.timeout
  )
}

// Package the crate this build script belongs to, into `$OUT_DIR/package.rs`.
pub fn package() {
  let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
  let manifest = Manifest::read(&root.join("service.toml")).unwrap_or_else(|error| panic!("{error}"));
  println!("cargo:rerun-if-changed={}", root.display());
  let version = version(&root).unwrap_or_else(|error| panic!("{error}"));
  let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("package.rs");
  std::fs::write(&out, generate(&manifest, &version)).unwrap_or_else(|error| panic!("Cannot write {}: {error}", out.display()));
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn embeds_the_name_version_binding_and_timeout() {
    let manifest = Manifest::parse("name = \"space\"\n[[sql]]\nbinding = \"db\"\n").unwrap();
    assert_eq!(
      generate(&manifest, "v1"),
      "pub const NAME: &str = \"space\";\npub const VERSION: &str = \"v1\";\npub const BINDING: Option<&str> = Some(\"db\");\npub const TIMEOUT: Option<u64> = None;\n"
    );
  }
}
