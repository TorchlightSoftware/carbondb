//! The TOML file an app hands CarbonDB, read once at startup:
//!
//! ```toml
//! system = "data/system.db"   # relative to this file; space files sit beside it
//! compiled = "data/compiled"  # where compiled components are kept; beside the system database by default
//!
//! [sync]
//! mode = "none"               # single-node; or "ref" (the default), "all", "manual"
//! relays = ["https://relay.example.org"]  # iroh relays every identity's node on this device uses; none by default
//! bootstrap = [{node = "<node id, hex>", relay = "https://relay.example.org"}]  # bootstraps offered; none by default
//! mdns = "active"             # find peers on the LAN: "active" (the default), "passive" or "off"
//! ```
//!
//! A relay named here serves every identity on this device, so whoever runs it
//! sees their nodes connect from the same address and can link them; a relay
//! device serves only its own identity. CarbonDB names no relay or bootstrap of
//! its own: on one LAN, mDNS is enough.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use p2panda::{NodeId, RelayUrl};
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
  pub network: Network,
}

// How this device's p2panda nodes reach peers: the relays every identity's
// node uses, the bootstraps offered to them, and whether they find peers on the
// LAN. Until an identity records its own choice, its node uses the bootstraps
// offered here.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
  pub relays: Vec<RelayUrl>,
  pub bootstrap: Vec<Bootstrap>,
  pub mdns: Mdns,
}

// A node to begin discovery from, and the relay that reaches it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bootstrap {
  pub node: NodeId,
  pub relay: RelayUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mdns {
  #[default]
  Active,
  Passive,
  Off,
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
  #[serde(default)]
  relays: Vec<String>,
  #[serde(default)]
  bootstrap: Vec<BootstrapEntry>,
  #[serde(default)]
  mdns: Mdns,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapEntry {
  node: String,
  relay: String,
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
    let network = network(&file.sync)?;
    Ok(Config { system, compiled, sync: file.sync.mode, network })
  }

  // The directory the system database and space files live in.
  pub fn directory(&self) -> &Path {
    self.system.parent().unwrap_or(Path::new("."))
  }
}

fn network(sync: &SyncSection) -> Result<Network> {
  let relays = sync.relays.iter().map(|relay| relay_url(relay)).collect::<Result<Vec<_>>>()?;
  let bootstrap = sync.bootstrap.iter().map(|entry| Ok(Bootstrap { node: node_id(&entry.node)?, relay: relay_url(&entry.relay)? })).collect::<Result<Vec<_>>>()?;
  Ok(Network { relays, bootstrap, mdns: sync.mdns })
}

fn relay_url(text: &str) -> Result<RelayUrl> {
  RelayUrl::from_str(text).map_err(|error| bad_request(format!("Malformed relay URL '{text}': {error}")))
}

fn node_id(text: &str) -> Result<NodeId> {
  NodeId::from_str(text).map_err(|error| bad_request(format!("Malformed node id '{text}': {error}")))
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
      assert_eq!(config, Config { system: PathBuf::from("/app/data/system.db"), compiled: PathBuf::from("/app/data/compiled"), sync, network: Network::default() });
      assert_eq!(config.directory(), Path::new("/app/data"));
    }
    let kept = Config::parse("system = \"data/system.db\"\ncompiled = \"/var/cache/carbondb\"", Path::new("/app")).unwrap();
    assert_eq!(kept.compiled, PathBuf::from("/var/cache/carbondb"));
  }

  #[test]
  fn reads_the_relays_bootstraps_and_mdns_nodes_use() {
    let node = crate::keys::generate().unwrap().public;
    let text = format!("system = \"s.db\"\n[sync]\nrelays = [\"https://relay.example.org\"]\nbootstrap = [{{node = \"{node}\", relay = \"https://relay.example.org\"}}]\nmdns = \"off\"");
    let network = Config::parse(&text, Path::new("/app")).unwrap().network;
    let relay = relay_url("https://relay.example.org").unwrap();
    assert_eq!(network, Network { relays: vec![relay.clone()], bootstrap: vec![Bootstrap { node: node_id(&node).unwrap(), relay }], mdns: Mdns::Off });
  }

  #[test]
  fn refuses_malformed_config() {
    let cases = [
      "",
      "system = 1",
      "system = \"s.db\"\n[sync]\nmode = \"sometimes\"",
      "system = \"s.db\"\nextra = true",
      "system = \"s.db\"\n[sync]\nrelays = [\"not a url\"]",
      "system = \"s.db\"\n[sync]\nbootstrap = [{node = \"nope\", relay = \"https://relay.example.org\"}]",
      "system = \"s.db\"\n[sync]\nmdns = \"loud\"",
    ];
    for text in cases {
      assert_eq!(Config::parse(text, Path::new("/app")).unwrap_err().status, 400, "{text}");
    }
  }
}
