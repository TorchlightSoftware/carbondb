//! The first milestone: the flow a person takes, two devices in one process,
//! in each network configuration: mDNS alone, as on one LAN; an iroh relay
//! the test runs on localhost, with mDNS off, the enrollment code standing in
//! for bootstrap; and a configured bootstrap, a third p2panda node behind the
//! same relay. The relay serves plain HTTP in dev mode, since p2panda's node
//! builder offers no way to trust a self-signed certificate.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use iroh_relay::server::{RelayConfig, Server, ServerConfig};

use super::*;
use crate::nodes::runtime;
use crate::testing::{awaited, found, on_awaited, register_script, syncing, texts, until_within};

// How long two devices may take to find each other and sync.
const NETWORK: Duration = Duration::from_secs(60);

// The texts of a channel's `[chat]` on a device, once it holds it.
fn chat(carbon: &Rc<crate::Carbon>, channel: &str) -> Vec<String> {
  let read = || -> crate::Result<Value> { carbon.read_tables(&carbon.named(channel, "chat")?, texts) };
  read().map(|texts| texts.as_array().unwrap().iter().map(|text| text.as_str().unwrap().to_string()).collect()).unwrap_or_default()
}

// The identities a device lists.
fn identities(carbon: &Rc<crate::Carbon>) -> Value {
  call_now(carbon, carbon.system(), &Actor::default(), "GET [@carbon]/identities", Value::Null).unwrap().value
}

// Create an identity on device 1, enroll device 2 in it, add [chat] to the
// personal channel, and chat between the two, both syncing as `sync` says.
fn flow(sync: &str) {
  let (_first, one) = syncing(sync);
  let (_second, two) = syncing(sync);
  register_script(&one, "chat");
  register_script(&two, "chat");

  // Device 1 founds alice; device 2 shows a code, which device 1 scans.
  let alice = found(&one, "alice", &[]);
  let shown = awaited(|answer| two.call(None, None, "POST [@carbon]/devices", Value::Null, answer)).unwrap();
  let enrolled = on_awaited(&one, &alice.channel, &alice, "POST [identity]/devices", json!({"code": shown["code"], "name": "Laptop", "tags": []})).unwrap();
  assert_eq!(enrolled["pubkey"], shown["device"]);

  // Device 2 joins alice, under its own device key.
  until_within(NETWORK, || identities(&two)[0]["id"] == json!(alice.id));
  assert_eq!(identities(&two)[0]["device"], shown["device"]);
  let laptop = crate::Identity { device: shown["device"].as_str().unwrap().to_string(), ..alice.clone() };

  // Alice adds [chat] to her personal channel, and each device's message reaches the other.
  on_awaited(&one, &alice.personal, &alice, "POST [channel]/services", json!({"name": "chat"})).unwrap();
  on_awaited(&one, &alice.personal, &alice, "POST [chat]/run", json!({"do": [{"insert": "from the phone"}]})).unwrap();
  until_within(NETWORK, || chat(&two, &laptop.personal) == ["from the phone"]);
  on_awaited(&two, &laptop.personal, &laptop, "POST [chat]/run", json!({"do": [{"insert": "from the laptop"}]})).unwrap();
  until_within(NETWORK, || chat(&one, &alice.personal) == ["from the laptop", "from the phone"]);
}

// An iroh relay on localhost, serving plain HTTP, and its URL.
fn relay() -> (Server, String) {
  let mut config = ServerConfig::default();
  config.relay = Some(RelayConfig::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))));
  let server = runtime().block_on(Server::spawn(config)).unwrap();
  let url = format!("http://{}", server.http_addr().unwrap());
  (server, url)
}

// A p2panda node behind a relay, as a bootstrap for others: its id.
fn bootstrap(relay: &str) -> (p2panda::Node, String) {
  let builder = p2panda::builder().mdns_mode(p2panda::network::MdnsDiscoveryMode::Disabled).relay_url(relay.parse().unwrap());
  let node = runtime().block_on(builder.spawn()).unwrap();
  let id = node.id().to_hex();
  (node, id)
}

#[test]
fn one_identity_enrolls_a_second_device_and_chats_between_them_over_mdns() {
  flow("mode = \"ref\"\nmdns = \"active\"");
}

#[test]
fn one_identity_enrolls_a_second_device_and_chats_between_them_through_a_relay_and_a_bootstrap() {
  let (_relay, url) = relay();
  let (_node, node) = bootstrap(&url);
  let configurations = [
    format!("mode = \"ref\"\nmdns = \"off\"\nrelays = [\"{url}\"]"),
    format!("mode = \"ref\"\nmdns = \"off\"\nrelays = [\"{url}\"]\nbootstrap = [{{node = \"{node}\", relay = \"{url}\"}}]"),
  ];
  configurations.iter().for_each(|sync| flow(sync));
}
