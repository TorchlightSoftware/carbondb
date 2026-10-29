# p2panda demo

A command-line, end-to-end encrypted group chat on p2panda's spaces API: the starting point for the six-step CarbonDB demo (create a room, invite with consent, offline merge, device enrollment, member removal, an always-on Pull-only node).

## Origin

`src/main.rs` began as a copy of p2panda's [`p2panda/examples/chat.rs`](https://github.com/p2panda/p2panda/blob/da80c05b95269926bf58121cf6a2d3dd5ca41dfc/p2panda/examples/chat.rs) at commit `da80c05b` (MIT OR Apache-2.0). Changes from upstream: logging set up with `tracing-subscriber` instead of p2panda's test utilities, `parse_action` made synchronous, and a data-driven parser test.

p2panda's spaces API is only on main after v0.7.1, so `Cargo.toml` pins `p2panda` and `p2panda-core` to that git commit. Move to crates.io versions once 0.8 is released.

## Run

Requires Rust 1.96 or later. From this directory:

```
# Terminal 1: creates the room, prints MEMBER ID and CHAT ID
cargo run

# Terminal 2: joins the room, prints its MEMBER ID, waits to be added
cargo run -- <CHAT_ID>

# Terminal 1, once it prints "KEY BUNDLE RECEIVED: <MEMBER_ID>" for terminal 2
add <MEMBER_ID>            # Write access
add <MEMBER_ID> manage     # Manage access
remove <MEMBER_ID>
```

Any other line is sent as a chat message. On one machine the instances find each other over mDNS. Across networks, pass any member's MEMBER ID as a bootstrap: `cargo run -- <CHAT_ID> <MEMBER_ID>`; traffic then goes through n0's public relay (`RELAY_URLS` in `src/main.rs`). Set `RUST_LOG=info` (or `debug`) for logs.

`add` fails with "missing key bundle" if it runs before the inviter has received the joiner's key bundle; wait for the `KEY BUNDLE RECEIVED` line.

## Test

```
cargo test
```
