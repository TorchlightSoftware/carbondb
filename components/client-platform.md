# Client Platform

Critical path: yes. Depends on: none.

## Context

- Not a wiki term: the client runtime and its bridge to the networking core.
- An earlier session explored KeyOxide + Flutter; KeyOxide's own app is Flutter (AGPL-3.0); aspe-dart is MIT.
- nostr: NDK 0.10.1 (MIT, 2026-10-02) and dart_nostr 10.0.1; 0xchat is a working Flutter app with MLS groups and Cashu.
- White Noise archived its Flutter app and is rebuilding natively in Kotlin and Swift on the Rust MDK core; the reason is unknown.
- iroh has official bindings for Python, Swift, Kotlin and Node; community iroh_flutter 1.3.0 (Apache-2.0) covers iroh core on mobile and desktop.
- p2panda has no Dart bindings (p2panda-ffi for Node, Python, Go is experimental), so our Rust core needs flutter_rust_bridge either way.
- Holochain: no Dart client; no mobile template; iOS only ran a diagnostic app; Android is experimental via the Tauri-based p2p Shipyard.
- Freenet: no mobile node and no Dart SDK; Rust and TypeScript clients talk to a node over WebSocket.
- Autonomi: Dart antd_client talks to a local antd daemon; Swift and Kotlin bindings via UniFFI; mobile SDKs in progress.
- Ockam: FFI and Swift layers were removed; no Dart support.
- Brandon intends networking and persistence in Rust, services in JS, Flutter calling in; proposal: Flutter calls the Rust core, which hosts the JS services (QuickJS via rquickjs or LLRT, since iOS forbids JIT), so one resolver serves UI, local and peer calls.

## Agreed

None yet.

## Open questions

- Is Flutter a firm choice, given the strongest group-crypto cores (MDK, OpenMLS) are Rust and White Noise left Flutter?
- Why did White Noise leave Flutter?
- Should the Rust core host the JS services, or should Flutter call JS directly?
