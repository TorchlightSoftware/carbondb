//! Package a Rust service built as a component: embed the `service.toml` its
//! directory holds, and record beside the package the hash of the sources it
//! was built from, so a check can tell when a committed package is stale.
//!
//!   carbondb-package <component.wasm> <service-dir> <out.wasm> [<source-dir>...]
//!
//! writes `<out.wasm>` and `<out>.source`. The sources are the service's
//! directory and any other directory its bytes depend on.

use std::path::{Path, PathBuf};

use carbondb_service_bindings::manifest::{Manifest, sources};
use carbondb_service_bindings::package::embed;

fn main() {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let [component, service, out, others @ ..] = args.as_slice() else {
    eprintln!("usage: carbondb-package <component.wasm> <service-dir> <out.wasm> [<source-dir>...]");
    std::process::exit(2);
  };
  if let Err(error) = package(Path::new(component), Path::new(service), Path::new(out), others) {
    eprintln!("{error}");
    std::process::exit(1);
  }
}

fn package(component: &Path, service: &Path, out: &Path, others: &[String]) -> Result<(), String> {
  let manifest_path = service.join("service.toml");
  let manifest = std::fs::read_to_string(&manifest_path).map_err(|error| format!("Cannot read {}: {error}", manifest_path.display()))?;
  Manifest::parse(&manifest)?;
  let bytes = std::fs::read(component).map_err(|error| format!("Cannot read {}: {error}", component.display()))?;
  write(out, &embed(&bytes, &manifest)?)?;
  let roots: Vec<PathBuf> = std::iter::once(service.to_path_buf()).chain(others.iter().map(PathBuf::from)).collect();
  write(&out.with_extension("source"), format!("{}\n", sources(&roots)?).as_bytes())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
  std::fs::write(path, bytes).map_err(|error| format!("Cannot write {}: {error}", path.display()))
}
