//! How long a new device waits for a space's first message, for the first space it holds and for
//! a second one it starts holding only after the first has synced, as CarbonDB chains a new
//! device's channels: alice's identity channel, then the personal channel it points to.
//!
//! Each run starts three nodes behind a relay on localhost: alice's phone, her new laptop, and an
//! unrelated node both are configured with as a bootstrap, which discovery's random walk may
//! visit. The phone records the laptop's address and adds it to two spaces, as enrolling does,
//! and publishes a message in each; the laptop then holds the first space, and once its message
//! arrives, the second.
//!
//! `cargo run --release -- <runs> [idle seconds]` measures p2panda-net as released; `cargo run
//! --release --features check-in -- <runs> [idle seconds]` with the check-in. The idle seconds
//! pass after the phone has set up both spaces and before the laptop holds the first, letting
//! discovery back off as on devices that have run a while. A third argument `foreign` runs the
//! bootstrap node on p2panda's default network instead, as CarbonDB's milestone test does by
//! mistake, so every discovery session with it fails.

use std::time::{Duration, Instant};

use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use p2panda::network::MdnsDiscoveryMode;
use p2panda::spaces::{AccessLevel, PendingSpace, Space, SpaceSubscription};
use p2panda::streams::StreamEvent;
use p2panda::{Hash, Node, NodeBuilder, NodeId, RelayUrl};
use tokio_stream::StreamExt;

// How long a wait may take before the run counts it as never arriving.
const GIVE_UP: Duration = Duration::from_secs(120);

type Error = Box<dyn std::error::Error>;

// A space the laptop holds for as long as this is kept.
type Held = (PendingSpace<String>, SpaceSubscription<String>);

// What one run measured: how long the laptop waited for each space's message, if it arrived.
struct Waits {
  first: Option<Duration>,
  chained: Option<Duration>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
  tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_writer(std::io::stderr).init();
  let runs: usize = std::env::args().nth(1).map_or(Ok(20), |runs| runs.parse())?;
  let idle = Duration::from_secs(std::env::args().nth(2).map_or(Ok(0), |idle| idle.parse())?);
  let foreign = std::env::args().nth(3).is_some_and(|bootstrap| bootstrap == "foreign");
  let relay = relay().await?;
  let url: RelayUrl = format!("http://{}", relay.http_addr().expect("the relay serves http")).parse()?;
  println!("check-in: {}, idle before the laptop holds a space: {}s, bootstrap on another network: {foreign}", cfg!(feature = "check-in"), idle.as_secs());
  let mut measured = vec![];
  for run in 1..=runs {
    let waits = waits(&url, idle, foreign).await?;
    println!("run {run:>2}: first space {}, chained space {}", shown(waits.first), shown(waits.chained));
    measured.push(waits);
  }
  println!("first space:   {}", summary(measured.iter().map(|waits| waits.first).collect()));
  println!("chained space: {}", summary(measured.iter().map(|waits| waits.chained).collect()));
  Ok(())
}

// An iroh relay on localhost, serving plain HTTP.
async fn relay() -> Result<Server, Error> {
  let mut config = ServerConfig::default();
  config.relay = Some(RelayConfig::new(std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0))));
  Ok(Server::spawn(config).await?)
}

// A node on this spike's network behind the relay, with mDNS off.
fn node(relay: &RelayUrl) -> NodeBuilder {
  p2panda::builder().network_id(Hash::digest(b"p2panda-check-in spike").into()).mdns_mode(MdnsDiscoveryMode::Disabled).relay_url(relay.clone())
}

async fn waits(relay: &RelayUrl, idle: Duration, foreign: bool) -> Result<Waits, Error> {
  let unrelated = if foreign { p2panda::builder().mdns_mode(MdnsDiscoveryMode::Disabled).relay_url(relay.clone()) } else { node(relay) }.spawn().await?;
  let (phone, laptop) = (node(relay).bootstrap(unrelated.id(), relay.clone()).spawn().await?, node(relay).bootstrap(unrelated.id(), relay.clone()).spawn().await?);
  enroll(&phone, &laptop, relay).await?;
  // The phone holds both spaces, and the laptop each it starts holding, until the run ends.
  let identity = space_with_message(&phone, laptop.id(), "identity").await?;
  let personal = space_with_message(&phone, laptop.id(), "personal").await?;
  // Nodes that have run a while have let discovery back off, as a device does before enrolling.
  tokio::time::sleep(idle).await;
  let (_identity, first) = first_message(&laptop, identity.0.id(), phone.id()).await?;
  let (_personal, chained) = match first {
    Some(_) => first_message(&laptop, personal.0.id(), phone.id()).await?,
    None => (None, None),
  };
  Ok(Waits { first, chained })
}

// The phone learns the laptop's key bundle and address, as scanning its code does.
async fn enroll(phone: &Node, laptop: &Node, relay: &RelayUrl) -> Result<(), Error> {
  phone.register_member(laptop.me().await?).await?;
  phone.insert_bootstrap(laptop.id(), relay.clone()).await?;
  Ok(())
}

// A space the phone creates and holds, the laptop a member of it, holding one message.
async fn space_with_message(phone: &Node, laptop: NodeId, text: &str) -> Result<(Space<String>, SpaceSubscription<String>), Error> {
  let (space, subscription) = phone.create_space::<String>().await?;
  space.add(laptop, AccessLevel::Write).await?;
  space.publish(text.to_string()).await?.await?;
  Ok((space, subscription))
}

// The laptop starts holding a space: the space it now holds, and how long until the phone's
// message arrived, if it did.
async fn first_message(laptop: &Node, space: Hash, phone: NodeId) -> Result<(Option<Held>, Option<Duration>), Error> {
  let started = Instant::now();
  let (pending, mut subscription) = laptop.space::<String>(space).await?;
  let arrived = tokio::time::timeout(GIVE_UP, from(&mut subscription, phone)).await;
  Ok((Some((pending, subscription)), arrived.ok().map(|_| started.elapsed())))
}

// Wait for a message the phone wrote.
async fn from(subscription: &mut SpaceSubscription<String>, phone: NodeId) {
  while let Some(event) = subscription.next().await {
    if let StreamEvent::Processed { operation, .. } = event
      && operation.author() == phone
    {
      return;
    }
  }
}

fn shown(wait: Option<Duration>) -> String {
  wait.map_or_else(|| format!("over {}s", GIVE_UP.as_secs()), |wait| format!("{:.2}s", wait.as_secs_f64()))
}

// The waits of every run, as their median, slowest and how many never arrived.
fn summary(waits: Vec<Option<Duration>>) -> String {
  let mut arrived: Vec<Duration> = waits.iter().flatten().copied().collect();
  arrived.sort();
  let missing = waits.len() - arrived.len();
  match (arrived.get(arrived.len() / 2), arrived.last()) {
    (Some(median), Some(slowest)) => format!("median {:.2}s, slowest {:.2}s, never arrived {missing}", median.as_secs_f64(), slowest.as_secs_f64()),
    _ => format!("never arrived {missing}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn summarizes_the_waits_of_every_run() {
    let seconds = |s: u64| Some(Duration::from_secs(s));
    let cases = [
      (vec![seconds(3), seconds(1), seconds(2)], "median 2.00s, slowest 3.00s, never arrived 0"),
      (vec![seconds(1), None], "median 1.00s, slowest 1.00s, never arrived 1"),
      (vec![None], "never arrived 1"),
    ];
    for (waits, expected) in cases {
      assert_eq!(summary(waits), expected);
    }
  }
}
