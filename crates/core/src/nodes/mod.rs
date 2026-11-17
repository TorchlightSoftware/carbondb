//! p2panda nodes: each identity on this device runs one, signing with its
//! device key, with a store file of its own beside the system database, since a
//! node takes one set of credentials and identities on one device must stay
//! unlinkable. A node runs whenever its identity is listed in the system
//! database and the device syncs at all.
//!
//! Every node in the process runs on one multi-threaded tokio runtime of its
//! own (`runtime`), not on the core's thread: that thread runs SQLite,
//! handlers and rewinds, which would stall the network's timers and sync
//! sessions. What a node does reaches the core as a task on the core's thread
//! awaiting it, the way any other thread's work wakes the core.

mod load;
mod members;
pub mod pairing;
mod publish;
pub mod receive;
pub(crate) mod spaces;

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use p2panda::network::MdnsDiscoveryMode;
use p2panda::node::AckPolicy;
use p2panda::{Credentials, Hash, Node, NodeBuilder};
use serde_json::Value;

use crate::carbon::Carbon;
use crate::config::{Mdns, Network, SyncMode};
use crate::error::{Result, ServiceError, internal};
use crate::services::carbon as local;

// The network CarbonDB's nodes form, apart from other p2panda applications'.
// It is not secret: spaces are encrypted towards their members.
const NETWORK: &str = "carbondb";

// The runtime every node in this process runs on, started on first use.
pub fn runtime() -> &'static tokio::runtime::Runtime {
  static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
  RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().thread_name("carbondb-p2panda").enable_all().build().expect("a tokio runtime for p2panda"))
}

// Run something on the nodes' runtime, and answer on the core's thread once it has.
pub(crate) fn on_network<T: Send + 'static>(carbon: &Rc<Carbon>, work: impl Future<Output = Result<T>> + Send + 'static, then: impl FnOnce(Result<T>) + 'static) {
  let running = runtime().spawn(work);
  let _ = carbon.spawn(Box::pin(async move {
    then(running.await.unwrap_or_else(|error| Err(internal(format!("A p2panda task failed: {error}")))))
  }));
}

// How a node that is starting reaches whoever asked for it.
pub type NodeReady = Box<dyn FnOnce(Result<Arc<Node>>)>;

// The nodes of the identities on this device, by identity id.
#[derive(Default)]
pub(crate) struct Nodes {
  by_identity: RefCell<HashMap<String, NodeState>>,
  // The p2panda spaces the nodes follow, by service space.
  spaces: spaces::Spaces,
  // The devices here waiting to join an identity.
  pairings: pairing::Pairings,
  // The channels loaded in the `ref` sync mode, whose refs and services are watched.
  loading: RefCell<std::collections::HashSet<String>>,
}

enum NodeState {
  Starting(Vec<NodeReady>),
  Running(Arc<Node>),
  Failed(ServiceError),
}

impl Carbon {
  // Run a node for each identity on this device, now and as identities are
  // added, by watching the system database's list of them; in the `ref` sync
  // mode, load what each identity's own channel reaches (`load`).
  pub(crate) fn run_nodes(self: &Rc<Self>) -> Result<()> {
    self.take_arrivals();
    let carbon = Rc::downgrade(self);
    let deliver = Rc::new(move |listed: Result<Value>| {
      if let (Some(carbon), Ok(listed)) = (carbon.upgrade(), listed) {
        for identity in listed.as_array().into_iter().flatten() {
          let (Some(id), Some(channel)) = (identity["id"].as_str(), identity["channel"].as_str()) else { continue };
          carbon.start_node(id);
          if carbon.config.sync == SyncMode::Ref {
            carbon.load_identity(id, channel);
          }
        }
      }
    });
    let system = self.system().id.clone();
    self.watch(&system, None, "GET [@carbon]/identities", deliver).map(|_| ())
  }

  // An identity's node, once it has started.
  pub fn with_node(&self, identity: &str, ready: NodeReady) {
    let mut nodes = self.nodes.by_identity.borrow_mut();
    match nodes.get_mut(identity) {
      Some(NodeState::Starting(waiting)) => waiting.push(ready),
      Some(NodeState::Running(node)) => {
        let node = Arc::clone(node);
        drop(nodes);
        ready(Ok(node))
      }
      Some(NodeState::Failed(error)) => {
        let error = error.clone();
        drop(nodes);
        ready(Err(error))
      }
      None => {
        drop(nodes);
        ready(Err(crate::error::not_found(format!("No node runs for identity {identity} on this device"))))
      }
    }
  }

  // Start an identity's node, unless it has one: its credentials from the
  // system database, its store beside it.
  fn start_node(self: &Rc<Self>, identity: &str) {
    if self.nodes.by_identity.borrow().contains_key(identity) {
      return;
    }
    match self.system.store().reader().and_then(|system| local::credentials(&system, identity)) {
      Ok(credentials) => self.spawn_node(identity, credentials),
      Err(error) => {
        self.nodes.by_identity.borrow_mut().insert(identity.to_string(), NodeState::Failed(error));
      }
    }
  }

  // Start the node of an identity being founded, from the secret of its
  // first device, before the identity is kept here: its node creates the
  // p2panda spaces of its channels. A single-node CarbonDB starts none.
  pub(crate) fn start_founding_node(self: &Rc<Self>, identity: &str, secret: &str) -> Result<()> {
    if self.config.sync == SyncMode::None {
      return Ok(());
    }
    self.spawn_node(identity, crate::keys::credentials(secret)?);
    Ok(())
  }

  // A node started under one key, now known by another, as a joining device's under its identity.
  pub(crate) fn rename_node(&self, from: &str, to: &str) {
    let mut nodes = self.nodes.by_identity.borrow_mut();
    if let Some(state) = nodes.remove(from) {
      nodes.insert(to.to_string(), state);
    }
  }

  // Stop an identity's node, as when founding the identity failed.
  pub(crate) fn stop_node(&self, identity: &str) {
    self.nodes.by_identity.borrow_mut().remove(identity);
  }

  pub(crate) fn spawn_node(self: &Rc<Self>, identity: &str, credentials: Credentials) {
    let builder = builder(credentials, &self.node_path(identity), &self.config.network);
    self.nodes.by_identity.borrow_mut().insert(identity.to_string(), NodeState::Starting(vec![]));
    let (carbon, identity) = (Rc::clone(self), identity.to_string());
    on_network(self, async move { builder.spawn().await.map_err(|error| internal(format!("Cannot start a p2panda node: {error}"))) }, move |started| {
      carbon.node_started(&identity, started.map(Arc::new))
    });
  }

  // A node has started, or failed to: whoever waits for it hears, unless it
  // was stopped meanwhile.
  fn node_started(self: &Rc<Self>, identity: &str, started: Result<Arc<Node>>) {
    let state = match &started {
      Ok(node) => NodeState::Running(Arc::clone(node)),
      Err(error) => NodeState::Failed(error.clone()),
    };
    let waiting = match self.nodes.by_identity.borrow_mut().get_mut(identity) {
      Some(starting @ NodeState::Starting(_)) => match std::mem::replace(starting, state) {
        NodeState::Starting(waiting) => waiting,
        _ => vec![],
      },
      _ => return,
    };
    waiting.into_iter().for_each(|ready| ready(started.clone()));
    if let (Ok(_), Err(error)) = (&started, self.rejoin(identity)) {
      eprintln!("[carbondb] the spaces of identity {identity} are not synced: {}", error.message);
    }
  }

  // Where an identity's node keeps its store: beside the system database.
  pub(crate) fn node_path(&self, identity: &str) -> PathBuf {
    self.config.directory().join(format!("{identity}.p2panda.db"))
  }
}

// A node for an identity: its credentials and store, CarbonDB's network,
// acknowledging each op once it is logged, and the relays, bootstraps and mDNS
// the config names.
fn builder(credentials: Credentials, store: &std::path::Path, network: &Network) -> NodeBuilder {
  let builder = p2panda::builder()
    .credentials(credentials)
    .database_url(&format!("sqlite://{}?mode=rwc", store.display()))
    .network_id(Hash::digest(NETWORK).into())
    .ack_policy(AckPolicy::Explicit)
    .mdns_mode(mdns_mode(network.mdns));
  let builder = network.relays.iter().fold(builder, |builder, relay| builder.relay_url(relay.clone()));
  network.bootstrap.iter().fold(builder, |builder, bootstrap| builder.bootstrap(bootstrap.node, bootstrap.relay.clone()))
}

fn mdns_mode(mdns: Mdns) -> MdnsDiscoveryMode {
  match mdns {
    Mdns::Active => MdnsDiscoveryMode::Active,
    Mdns::Passive => MdnsDiscoveryMode::Passive,
    Mdns::Off => MdnsDiscoveryMode::Disabled,
  }
}
