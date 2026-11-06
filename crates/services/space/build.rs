// Embed this service's service.toml name, version, sql binding and migrations.
fn main() {
  carbondb_service_bindings::build::package();
}
