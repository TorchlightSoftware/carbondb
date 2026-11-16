//! The TOML file an app hands CarbonDB, read once at startup:
//!
//! ```toml
//! system = "data/system.db"   # relative to this file; space files sit beside it
//! compiled = "data/compiled"  # where compiled components are kept; beside the system database by default
//!
//! [sync]
//! mode = "none"               # single-node; or "ref" (the default), "all", "manual"
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Result, bad_request};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
  #[default]
  Ref,
  All,
  Manual,
  None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
  pub system: PathBuf,
  // Where compiled components are kept, for every CarbonDB the process opens
  // after this one too (`crate::wasm::keep_in`).
  pub compiled: PathBuf,
  pub sync: SyncMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
  system: String,
  compiled: Option<String>,
  #[serde(default)]
  sync: SyncSection,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SyncSection {
  #[serde(default)]
  mode: SyncMode,
}

impl Config {
  pub fn load(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path).map_err(|error| bad_request(format!("Cannot read {}: {error}", path.display())))?;
    Config::parse(&text, path.parent().unwrap_or(Path::new(".")))
  }

  // A config from TOML text, with paths resolved against the file's directory.
  pub fn parse(text: &str, base: &Path) -> Result<Config> {
    let file: File = toml::from_str(text).map_err(|error| bad_request(format!("Malformed config: {error}")))?;
    let system = base.join(file.system);
    let compiled = file.compiled.map_or_else(|| system.parent().unwrap_or(base).join("compiled"), |compiled| base.join(compiled));
    Ok(Config { system, compiled, sync: file.sync.mode })
  }

  // The directory the system database and space files live in.
  pub fn directory(&self) -> &Path {
    self.system.parent().unwrap_or(Path::new("."))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_the_system_path_where_compiled_components_are_kept_and_the_sync_mode() {
    let cases = [
      ("system = \"data/system.db\"\n[sync]\nmode = \"none\"", SyncMode::None),
      ("system = \"data/system.db\"", SyncMode::Ref),
      ("system = \"data/system.db\"\n[sync]\nmode = \"manual\"", SyncMode::Manual),
    ];
    for (text, sync) in cases {
      let config = Config::parse(text, Path::new("/app")).unwrap();
      assert_eq!(config, Config { system: PathBuf::from("/app/data/system.db"), compiled: PathBuf::from("/app/data/compiled"), sync });
      assert_eq!(config.directory(), Path::new("/app/data"));
    }
    let kept = Config::parse("system = \"data/system.db\"\ncompiled = \"/var/cache/carbondb\"", Path::new("/app")).unwrap();
    assert_eq!(kept.compiled, PathBuf::from("/var/cache/carbondb"));
  }

  #[test]
  fn refuses_malformed_config() {
    for text in ["", "system = 1", "system = \"s.db\"\n[sync]\nmode = \"sometimes\"", "system = \"s.db\"\nextra = true"] {
      assert_eq!(Config::parse(text, Path::new("/app")).unwrap_err().status, 400, "{text}");
    }
  }
}
