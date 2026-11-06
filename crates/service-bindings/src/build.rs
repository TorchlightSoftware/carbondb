//! Packaging a Rust service, at build time, behind the `build` feature. Its
//! build script calls `package()`, which reads the crate's `service.toml` and the
//! migrations its `sql` binding names, and writes what the crate embeds: its
//! name, its version, its `sql` binding and its migrations, for `init` to apply.
//!
//! ```toml
//! name = "space"
//!
//! [[sql]]
//! binding = "db"
//! migrations_dir = "migrations"   # its .sql files, applied in file name order
//! ```
//!
//! The version is the hash of the package: its service.toml, its migrations and
//! its source, so different code is a different version.

use std::path::{Path, PathBuf};

use crate::hash::hash;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
  pub name: String,
  #[serde(default)]
  pub sql: Vec<Sql>,
}

// A `sql` binding: the storage a service asks for, and the migrations that shape it.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sql {
  pub binding: String,
  pub migrations_dir: String,
}

impl Manifest {
  pub fn parse(text: &str) -> Result<Manifest, String> {
    let manifest: Manifest = toml::from_str(text).map_err(|error| format!("Malformed service.toml: {error}"))?;
    if manifest.sql.len() > 1 {
      return Err("A service declares at most one sql binding, since a space keeps every service's tables in one file".into());
    }
    Ok(manifest)
  }

  // The migration files, in the order they apply.
  pub fn migrations(&self, root: &Path) -> Result<Vec<PathBuf>, String> {
    let Some(sql) = self.sql.first() else { return Ok(vec![]) };
    files(&root.join(&sql.migrations_dir), "sql")
  }
}

// The files with an extension under a directory, at any depth, in path order.
fn files(directory: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
  let entries = std::fs::read_dir(directory).map_err(|error| format!("Cannot read {}: {error}", directory.display()))?;
  let mut found = vec![];
  for path in entries.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
    if path.is_dir() {
      found.extend(files(&path, extension)?);
    } else if path.extension().is_some_and(|candidate| candidate == extension) {
      found.push(path);
    }
  }
  found.sort();
  Ok(found)
}

// The package's version: the hash of every file that makes it, by its path within the crate.
pub fn version(root: &Path, files: &[PathBuf]) -> Result<String, String> {
  let contents: Vec<_> = files
    .iter()
    .map(|path| {
      let text = std::fs::read_to_string(path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
      Ok(json!([path.strip_prefix(root).unwrap_or(path).display().to_string(), text]))
    })
    .collect::<Result<_, String>>()?;
  Ok(hash(&json!(contents)))
}

// The Rust a package embeds: its name, version, sql binding, and each migration's name and text.
pub fn generate(manifest: &Manifest, version: &str, migrations: &[PathBuf]) -> String {
  let binding = manifest.sql.first().map(|sql| sql.binding.as_str());
  let included: Vec<String> = migrations
    .iter()
    .map(|path| {
      let name = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
      format!("({name:?}, include_str!({:?}))", path.display().to_string())
    })
    .collect();
  format!(
    "pub const NAME: &str = {:?};\npub const VERSION: &str = {version:?};\npub const BINDING: Option<&str> = {binding:?};\npub const MIGRATIONS: &[(&str, &str)] = &[{}];\n",
    manifest.name,
    included.join(", ")
  )
}

// Package the crate this build script belongs to, into `$OUT_DIR/package.rs`.
pub fn package() {
  let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
  let manifest_path = root.join("service.toml");
  let text = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| panic!("Cannot read {}: {error}", manifest_path.display()));
  let manifest = Manifest::parse(&text).unwrap_or_else(|error| panic!("{error}"));
  let migrations = manifest.migrations(&root).unwrap_or_else(|error| panic!("{error}"));
  let sources = files(&root.join("src"), "rs").unwrap_or_else(|error| panic!("{error}"));
  let made_of: Vec<PathBuf> = std::iter::once(manifest_path).chain(migrations.iter().cloned()).chain(sources).collect();
  for path in &made_of {
    println!("cargo:rerun-if-changed={}", path.display());
  }
  for sql in &manifest.sql {
    println!("cargo:rerun-if-changed={}", root.join(&sql.migrations_dir).display());
  }
  let version = version(&root, &made_of).unwrap_or_else(|error| panic!("{error}"));
  let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("package.rs");
  std::fs::write(&out, generate(&manifest, &version, &migrations)).unwrap_or_else(|error| panic!("Cannot write {}: {error}", out.display()));
}

#[cfg(test)]
mod tests {
  use super::*;

  const SPACE: &str = "name = \"space\"\n[[sql]]\nbinding = \"db\"\nmigrations_dir = \"migrations\"\n";

  #[test]
  fn reads_a_name_and_at_most_one_sql_binding() {
    let manifest = Manifest::parse(SPACE).unwrap();
    assert_eq!(manifest, Manifest { name: "space".into(), sql: vec![Sql { binding: "db".into(), migrations_dir: "migrations".into() }] });
    assert_eq!(Manifest::parse("name = \"carbon\"\n").unwrap().sql, vec![]);
    let refused = [
      "",
      "name = \"space\"\nvars = {}\n",
      "name = \"space\"\n[[sql]]\nbinding = \"db\"\nmigrations_dir = \"m\"\nextra = 1\n",
      "name = \"space\"\n[[sql]]\nbinding = \"a\"\nmigrations_dir = \"m\"\n[[sql]]\nbinding = \"b\"\nmigrations_dir = \"m\"\n",
    ];
    for text in refused {
      assert!(Manifest::parse(text).is_err(), "{text}");
    }
  }

  #[test]
  fn lists_migrations_in_file_name_order_and_embeds_them_by_name() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("migrations")).unwrap();
    for file in ["0002_add_tags.sql", "0001_create.sql", "notes.md"] {
      std::fs::write(root.path().join("migrations").join(file), "").unwrap();
    }
    let manifest = Manifest::parse(SPACE).unwrap();
    let migrations = manifest.migrations(root.path()).unwrap();
    let names: Vec<_> = migrations.iter().map(|path| path.file_name().unwrap().to_str().unwrap()).collect();
    assert_eq!(names, ["0001_create.sql", "0002_add_tags.sql"]);
    let code = generate(&manifest, "v1", &migrations);
    assert!(code.starts_with("pub const NAME: &str = \"space\";\npub const VERSION: &str = \"v1\";\npub const BINDING: Option<&str> = Some(\"db\");\n"));
    assert!(code.contains("(\"0001_create\", include_str!("));
    assert_eq!(code.matches("include_str!").count(), 2);
  }

  #[test]
  fn the_version_changes_with_any_file_that_makes_the_package() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("lib.rs");
    std::fs::write(&file, "fn a() {}").unwrap();
    let before = version(root.path(), std::slice::from_ref(&file)).unwrap();
    assert_eq!(before, version(root.path(), std::slice::from_ref(&file)).unwrap());
    std::fs::write(&file, "fn b() {}").unwrap();
    assert_ne!(before, version(root.path(), &[file]).unwrap());
  }
}
