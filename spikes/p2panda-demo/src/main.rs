// SPDX-License-Identifier: MIT OR Apache-2.0

//! Example E2EE group chat CLI app using the spaces API.
//!
//! ## Usage
//!
//! Run the example in one terminal to create a new chat:
//!
//! `cargo run`
//!
//! Then using the CHAT_ID output from the first instance for the <CHAT_ID> argument, run as many
//! more instances as you like. The <BOOTSTRAP> argument is optional and only required if
//! discovery should run over the internet. Any other member's MEMBER_ID can be used as a
//! bootstrap.
//!
//! `cargo run -- <CHAT_ID> <BOOTSTRAP>`
//!
//! ### Commands
//!
//! ```text
//! # add a member to the chat
//! add <MEMBER_ID>
//!
//! # add a member to the chat with manager rights
//! add <MEMBER_ID> manage
//!
//! # remove a member from the chat
//! remove <MEMBER_ID>
//! ```
use std::collections::VecDeque;
use std::str::FromStr;
use std::thread;

use p2panda::spaces::SpaceSubscription;
use p2panda::streams::StreamEvent;
use p2panda::{AccessLevel, Hash, RelayUrl, SpaceEvent, Topic, VerifyingKey};
use p2panda_core::IdentityError;
use p2panda_core::traits::ShortFormat;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

type Message = String;

const RELAY_URLS: &[&str] = &["https://euc1-1.relay.n0.iroh.link/."];

const NETWORK_ID: &str = "chat";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_logging();

    let args: Vec<String> = std::env::args().collect();

    let space_id = if args.len() > 1 {
        let topic = Topic::from_str(&args[1]).map_err(|err| format!("invalid space id: {err}"))?;
        Some(topic)
    } else {
        None
    };

    let bootstrap = if args.len() > 2 {
        let bootstrap =
            VerifyingKey::from_str(&args[2]).map_err(|err| format!("invalid bootstrap: {err}"))?;
        Some(bootstrap)
    } else {
        None
    };

    // Build and spawn the node, adding an optional bootstrap which is required for discovery over
    // the internet with the help of a relay. Any <MEMBER_ID> can be used as the bootstrap.
    let node = {
        let mut builder = p2panda::builder().network_id(Hash::digest(NETWORK_ID).into());

        if let Some(bootstrap) = bootstrap {
            for url in RELAY_URLS {
                let relay_url: RelayUrl = url.parse().unwrap();

                builder = builder
                    .relay_url(relay_url.clone())
                    .bootstrap(bootstrap, relay_url);
            }
        }

        builder.spawn().await?
    };

    println!("MEMBER ID: {}", node.id().to_hex());

    // If a space id was provided subscribe to the existing space and wait until we've been added,
    // otherwise create a new one. Share the id <CHAT ID> with other instances to allow them to join.
    let space = match space_id {
        Some(space_id) => {
            let (pending_space, space_rx) = node.space::<Message>(space_id).await?;

            println!("CHAT ID: {}", pending_space.id().to_hex());
            spawn_event_handler(space_rx);

            println!("WAITING TO BE ADDED ...");
            let space = pending_space.joined().await?;

            println!("JOINED SPACE!");
            space
        }
        None => {
            let space_id = Topic::random();
            let (space, space_rx) = node.create_space::<Message>(space_id).await?;
            println!("CHAT ID: {}", space.id().to_hex());
            spawn_event_handler(space_rx);

            space
        }
    };

    // Listen for text input via the terminal.
    let (line_tx, mut line_rx) = mpsc::channel(1);
    thread::spawn(move || input_loop(line_tx));

    // Parse text commands into space membership actions and call related space API methods.
    while let Some(str) = line_rx.recv().await {
        let action = match parse_action(str) {
            Ok(action) => action,
            Err(err) => {
                println!("invalid command: {err}");
                continue;
            }
        };

        match action {
            // Add a new member to the space.
            Action::Add { member, access } => {
                if let Err(err) = space.add(member, access).await {
                    println!("add member error: {err}");
                }
            }
            // Remove an existing member from the space.
            Action::Remove { member } => {
                if let Err(err) = space.remove(member).await {
                    println!("remove member error: {err}");
                }
            }
            // Publish a message to the space.
            Action::Message(message) => {
                let result = space.publish(message).await;
                let ready = match result {
                    Ok(ready) => ready,
                    Err(err) => {
                        println!("publish message error: {err}");
                        continue;
                    }
                };

                if let Err(err) = ready.await {
                    println!("await ready error: {err}");
                };

                continue;
            }
        };
    }

    Ok(())
}

/// Log to stderr, filtered by RUST_LOG (errors only when unset).
fn setup_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
}

/// Spawn process for handling events arriving on the space stream. Both events resulting from our
/// own actions as well as those from other members are emitted here.
fn spawn_event_handler(mut space_rx: SpaceSubscription<Message>) {
    tokio::task::spawn(async move {
        while let Some(event) = space_rx.next().await {
            match event {
                // Chat message.
                StreamEvent::Processed { operation, .. } => {
                    let message = operation.message();
                    println!("{}: {}", operation.author().fmt_short(), message);
                }
                // Space membership change.
                StreamEvent::Space { members, inner, .. } => {
                    if let SpaceEvent::Ejected { .. } = inner {
                        println!("YOU WERE REMOVED");
                        continue;
                    };

                    let members = members
                        .iter()
                        .map(|(member, _)| member.fmt_short())
                        .collect::<Vec<_>>()
                        .join(", ");

                    println!("MEMBERS: [{}]", members)
                }
                // Key bundle.
                StreamEvent::Member(actor) => {
                    println!("KEY BUNDLE RECEIVED: {}", actor)
                }
                _ => (),
            }
        }
    });
}

fn input_loop(line_tx: mpsc::Sender<String>) -> Result<(), std::io::Error> {
    let mut buffer = String::new();
    let stdin = std::io::stdin();
    loop {
        stdin.read_line(&mut buffer)?;
        line_tx
            .blocking_send(buffer.trim().to_string())
            .map_err(std::io::Error::other)?;
        buffer.clear();
    }
}

/// Supported space actions.
#[derive(Debug, PartialEq)]
enum Action {
    Add {
        member: VerifyingKey,
        access: AccessLevel,
    },
    Remove {
        member: VerifyingKey,
    },
    Message(String),
}

/// Parse CLI text into an action.
fn parse_action(str: String) -> Result<Action, ParseActionError> {
    if let Some(str) = str.strip_prefix("add") {
        let str = str.trim();
        let mut args: VecDeque<&str> = str.split(" ").filter(|s| !s.is_empty()).collect();
        let Some(member) = args.pop_front() else {
            return Err(ParseActionError::InvalidArgs(
                "member_id required when adding member to a group".to_string(),
            ));
        };

        let access = parse_access_level(args.pop_front())?;
        Ok(Action::Add {
            member: member.parse()?,
            access,
        })
    } else if let Some(str) = str.strip_prefix("remove") {
        let str = str.trim();
        let mut args: VecDeque<&str> = str.split(" ").filter(|s| !s.is_empty()).collect();
        let Some(member) = args.pop_front() else {
            return Err(ParseActionError::InvalidArgs(
                "member_id required when removing member from a group".to_string(),
            ));
        };
        Ok(Action::Remove {
            member: member.parse()?,
        })
    } else {
        Ok(Action::Message(str))
    }
}

fn parse_access_level(str: Option<&str>) -> Result<AccessLevel, ParseActionError> {
    let access = match str {
        None => AccessLevel::Write,
        Some("manage") => AccessLevel::Manage,
        Some(str) => return Err(ParseActionError::UnknownAccessLevel(str.to_string())),
    };
    Ok(access)
}

#[derive(Debug, Error)]
enum ParseActionError {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("unknown access level: {0}")]
    UnknownAccessLevel(String),

    #[error(transparent)]
    Identity(#[from] IdentityError),
}

#[cfg(test)]
mod tests {
    use p2panda::SigningKey;

    use super::*;

    // Each terminal line maps to the action it should produce, or the error it should raise.
    #[test]
    fn parses_terminal_lines_into_actions() {
        let member = SigningKey::generate().verifying_key();
        let id = member.to_hex();

        let cases: Vec<(String, Result<Action, &str>)> = vec![
            (format!("add {id}"), Ok(Action::Add { member, access: AccessLevel::Write })),
            (format!("add {id} manage"), Ok(Action::Add { member, access: AccessLevel::Manage })),
            (format!("add  {id}  "), Ok(Action::Add { member, access: AccessLevel::Write })),
            (format!("remove {id}"), Ok(Action::Remove { member })),
            ("hello panda".to_string(), Ok(Action::Message("hello panda".to_string()))),
            ("add".to_string(), Err("invalid arguments")),
            ("remove".to_string(), Err("invalid arguments")),
            (format!("add {id} read"), Err("unknown access level")),
            ("add not-a-key".to_string(), Err("")),
        ];

        for (line, expected) in cases {
            match (parse_action(line.clone()), expected) {
                (Ok(action), Ok(expected)) => assert_eq!(action, expected, "line: {line}"),
                (Err(err), Err(prefix)) => {
                    assert!(err.to_string().starts_with(prefix), "line: {line}, error: {err}")
                }
                (actual, expected) => panic!("line: {line}, got {actual:?}, expected {expected:?}"),
            }
        }
    }
}
