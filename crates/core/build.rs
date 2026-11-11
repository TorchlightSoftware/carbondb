// Embed each builtin service's service.toml name, version, sql binding and
// timeout. Each service's version is the hash of its own directory.
fn main() {
  for service in ["space", "identity", "device"] {
    carbondb_service_bindings::build::package(&format!("src/services/{service}"));
  }
}
