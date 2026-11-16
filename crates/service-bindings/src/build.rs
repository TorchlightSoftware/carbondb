//! Packaging a Rust service, at build time, behind the `build` feature. A
//! service is a directory holding its `service.toml`, migrations and source:
//! a crate of its own, or a module of a crate that ships several, as the core
//! ships its builtins. The crate's build script calls `package()` once per
//! service, which reads its `service.toml` and writes what the service embeds:
//! its name, its version, its `sql` binding and its timeout, into
//! `$OUT_DIR/<name>_package.rs`. How it sets up its storage is its own
//! `init`'s concern.
//!
//! The version is the hash of every file in the service's directory (see
//! `manifest`).

use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, version};

// The Rust a package embeds: its name, version, sql binding, timeout, and
// the effects it makes again on every run.
pub fn generate(manifest: &Manifest, version: &str) -> String {
  format!(
    "pub const NAME: &str = {:?};\npub const VERSION: &str = {version:?};\npub const BINDING: Option<&str> = {:?};\npub const TIMEOUT: Option<u64> = {:?};\npub const ALWAYS: &[&str] = &{:?};\n",
    manifest.name,
    manifest.binding(),
    manifest.timeout,
    manifest.replicated_always()
  )
}

// Package the service in `dir`, relative to the crate this build script
// belongs to: "." for a crate that is one service.
pub fn package(dir: &str) {
  let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join(dir);
  let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
  println!("cargo:rerun-if-changed={}", root.display());
  write(&root, &out).unwrap_or_else(|error| panic!("{error}"));
}

// Write what the service in `root` embeds into `out`, answering the file written.
pub fn write(root: &Path, out: &Path) -> Result<PathBuf, String> {
  let manifest = Manifest::read(&root.join("service.toml"))?;
  let file = out.join(format!("{}_package.rs", manifest.name));
  std::fs::write(&file, generate(&manifest, &version(root)?)).map_err(|error| format!("Cannot write {}: {error}", file.display()))?;
  Ok(file)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn embeds_the_name_version_binding_and_timeout() {
    let manifest = Manifest::parse("name = \"space\"\n[[sql]]\nbinding = \"db\"\n").unwrap();
    assert_eq!(
      generate(&manifest, "v1"),
      "pub const NAME: &str = \"space\";\npub const VERSION: &str = \"v1\";\npub const BINDING: Option<&str> = Some(\"db\");\npub const TIMEOUT: Option<u64> = None;\npub const ALWAYS: &[&str] = &[];\n"
    );
  }

  #[test]
  fn packages_a_service_directory_into_a_file_named_for_the_service() {
    let service = tempfile::tempdir().unwrap();
    std::fs::write(service.path().join("service.toml"), "name = \"space\"\n[[sql]]\nbinding = \"db\"\n").unwrap();
    std::fs::write(service.path().join("mod.rs"), "// the space service\n").unwrap();
    let out = tempfile::tempdir().unwrap();
    let written = write(service.path(), out.path()).unwrap();
    assert_eq!(written, out.path().join("space_package.rs"));
    let expected = generate(&Manifest::read(&service.path().join("service.toml")).unwrap(), &version(service.path()).unwrap());
    assert_eq!(std::fs::read_to_string(written).unwrap(), expected);
  }

  #[test]
  fn refuses_a_directory_without_a_service_toml() {
    let empty = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    assert!(write(empty.path(), out.path()).is_err());
  }
}
