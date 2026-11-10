//! A package's `service.toml`, CarbonDB's own manifest, and the package's
//! version, behind the `manifest` feature: a Rust package's build reads them
//! at build time, and the core reads an npm package's when it is registered.
//!
//! ```toml
//! name = "space"     # its canonical name; the name a host registers it under overrides it
//! timeout = 200      # milliseconds every call to it must settle within; the core defaults it
//!
//! [[sql]]
//! binding = "db"     # the storage it asks for; how it shapes it is its own `init`'s concern
//! ```
//!
//! The version is the hash of every file in the package, so different code,
//! migrations or manifest is a different version.

use std::path::Path;

use serde::Deserialize;

use crate::hash::to_hex;

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
  pub name: String,
  #[serde(default)]
  pub timeout: Option<u64>,
  #[serde(default)]
  pub sql: Vec<Sql>,
}

// A `sql` binding: the storage a service asks for, by name.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sql {
  pub binding: String,
}

impl Manifest {
  pub fn parse(text: &str) -> Result<Manifest, String> {
    let manifest: Manifest = toml::from_str(text).map_err(|error| format!("Malformed service.toml: {error}"))?;
    if manifest.sql.len() > 1 {
      return Err("A service declares at most one sql binding, since a space keeps every service's tables in one file".into());
    }
    Ok(manifest)
  }

  // The manifest a package keeps at this path.
  pub fn read(path: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    Manifest::parse(&text)
  }

  // The name of its one `sql` binding, if it asks for storage.
  pub fn binding(&self) -> Option<&str> {
    self.sql.first().map(|sql| sql.binding.as_str())
  }
}

// What a package is made of: every file under its directory, by its path
// within it, in path order. Installed dependencies are not the package's own.
pub fn files(root: &Path) -> Result<Vec<String>, String> {
  let mut found = vec![];
  gather(root, root, &mut found)?;
  found.sort();
  Ok(found)
}

fn gather(root: &Path, directory: &Path, found: &mut Vec<String>) -> Result<(), String> {
  let entries = std::fs::read_dir(directory).map_err(|error| format!("Cannot read {}: {error}", directory.display()))?;
  for path in entries.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
    if path.file_name().is_some_and(|name| name == "node_modules") {
      continue;
    }
    if path.is_dir() {
      gather(root, &path, found)?;
    } else {
      let within = path.strip_prefix(root).unwrap_or(&path);
      found.push(within.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
    }
  }
  Ok(())
}

// The package's version: the BLAKE3 hash of each of its files, its path and
// its bytes, in path order.
pub fn version(root: &Path) -> Result<String, String> {
  let mut hasher = blake3::Hasher::new();
  for file in files(root)? {
    let bytes = std::fs::read(root.join(&file)).map_err(|error| format!("Cannot read {file}: {error}"))?;
    for part in [file.as_bytes(), &bytes] {
      hasher.update(&(part.len() as u64).to_le_bytes());
      hasher.update(part);
    }
  }
  Ok(to_hex(hasher.finalize().as_bytes()))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_a_name_a_timeout_and_at_most_one_sql_binding_by_name() {
    let manifest = Manifest::parse("name = \"space\"\ntimeout = 50\n[[sql]]\nbinding = \"db\"\n").unwrap();
    assert_eq!(manifest, Manifest { name: "space".into(), timeout: Some(50), sql: vec![Sql { binding: "db".into() }] });
    assert_eq!(manifest.binding(), Some("db"));
    let bare = Manifest::parse("name = \"carbon\"\n").unwrap();
    assert_eq!((bare.timeout, bare.binding()), (None, None));
    let refused = [
      "",
      "name = \"space\"\nvars = {}\n",
      "name = \"space\"\ntimeout = \"200ms\"\n",
      "name = \"space\"\n[[sql]]\nbinding = \"db\"\nmigrations_dir = \"migrations\"\n",
      "name = \"space\"\n[[sql]]\nbinding = \"a\"\n[[sql]]\nbinding = \"b\"\n",
    ];
    for text in refused {
      assert!(Manifest::parse(text).is_err(), "{text}");
    }
  }

  #[test]
  fn the_version_changes_with_any_file_in_the_package_but_not_its_installed_dependencies() {
    let root = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
      let path = root.path().join(path);
      std::fs::create_dir_all(path.parent().unwrap()).unwrap();
      std::fs::write(path, text).unwrap();
    };
    write("service.toml", "name = \"chat\"\n");
    write("migrations/0001_create.sql", "CREATE TABLE a (id TEXT PRIMARY KEY) WITHOUT ROWID");
    write("node_modules/zod/index.js", "one");
    assert_eq!(files(root.path()).unwrap(), ["migrations/0001_create.sql", "service.toml"]);
    let before = version(root.path()).unwrap();
    assert_eq!(before, version(root.path()).unwrap());
    write("node_modules/zod/index.js", "two");
    assert_eq!(before, version(root.path()).unwrap());
    write("migrations/0001_create.sql", "CREATE TABLE b (id TEXT PRIMARY KEY) WITHOUT ROWID");
    assert_ne!(before, version(root.path()).unwrap());
  }
}
