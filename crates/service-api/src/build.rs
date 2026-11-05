//! Packaging a Rust service. Its build script calls `package()`, which reads
//! the crate's `service.toml` and the migrations its `sql` binding names, and
//! writes the name and migrations for the crate to embed:
//!
//! ```toml
//! name = "space"
//!
//! [[sql]]
//! binding = "db"
//! migrations_dir = "migrations"   # its .sql files, applied in file name order
//! ```
//!
//! So a service's SQL lives beside it as files, and its manifest is the one
//! place they are listed.

use std::path::{Path, PathBuf};

use serde::Deserialize;

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
    let directory = root.join(&sql.migrations_dir);
    let entries = std::fs::read_dir(&directory).map_err(|error| format!("Cannot read {}: {error}", directory.display()))?;
    let mut files: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| path.extension().is_some_and(|extension| extension == "sql")).collect();
    files.sort();
    Ok(files)
  }
}

// The Rust a package embeds: its name, and each migration's text.
pub fn generate(name: &str, migrations: &[PathBuf]) -> String {
  let included: Vec<String> = migrations.iter().map(|path| format!("include_str!({:?})", path.display().to_string())).collect();
  format!("pub const NAME: &str = {name:?};\npub const MIGRATIONS: &[&str] = &[{}];\n", included.join(", "))
}

// Package the crate this build script belongs to, into `$OUT_DIR/package.rs`.
pub fn package() {
  let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
  let manifest_path = root.join("service.toml");
  println!("cargo:rerun-if-changed={}", manifest_path.display());
  let text = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| panic!("Cannot read {}: {error}", manifest_path.display()));
  let manifest = Manifest::parse(&text).unwrap_or_else(|error| panic!("{error}"));
  for sql in &manifest.sql {
    println!("cargo:rerun-if-changed={}", root.join(&sql.migrations_dir).display());
  }
  let migrations = manifest.migrations(&root).unwrap_or_else(|error| panic!("{error}"));
  let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("package.rs");
  std::fs::write(&out, generate(&manifest.name, &migrations)).unwrap_or_else(|error| panic!("Cannot write {}: {error}", out.display()));
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_a_name_and_at_most_one_sql_binding() {
    let manifest = Manifest::parse("name = \"space\"\n[[sql]]\nbinding = \"db\"\nmigrations_dir = \"migrations\"\n").unwrap();
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
  fn lists_migrations_in_file_name_order_and_embeds_them() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("migrations")).unwrap();
    for file in ["002.sql", "001.sql", "notes.md"] {
      std::fs::write(root.path().join("migrations").join(file), "").unwrap();
    }
    let manifest = Manifest::parse("name = \"space\"\n[[sql]]\nbinding = \"db\"\nmigrations_dir = \"migrations\"\n").unwrap();
    let migrations = manifest.migrations(root.path()).unwrap();
    let names: Vec<_> = migrations.iter().map(|path| path.file_name().unwrap().to_str().unwrap()).collect();
    assert_eq!(names, ["001.sql", "002.sql"]);
    let code = generate("space", &migrations);
    assert!(code.starts_with("pub const NAME: &str = \"space\";\npub const MIGRATIONS: &[&str] = &[include_str!("));
    assert_eq!(code.matches("include_str!").count(), 2);
  }
}
