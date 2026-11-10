// Embed this service's service.toml name, version, sql binding and timeout.
fn main() {
  carbondb_service_bindings::build::package();
}
