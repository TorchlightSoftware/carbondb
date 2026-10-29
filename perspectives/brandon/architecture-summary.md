Brandon: Ok, I'm going to share my review of your platform assessments, and the feature matrix/comparison.

# Platforms

## Autonomi

Sounds like they are mostly solving "own your own data" but not a general purpose compute platform, e.g. where do my apps live and on what architecture.  Might be useful as a storage module, but otherwise doesn't solve the core problems that we want to solve.

## Freenet

Freenet is thinking along similar lines as I am, and I think has some interesting concepts and developments.  To be clear, for our use case I'm not asking for it to be the right platform for implementing a spending token.  I only expect it to establish identity and communication channels... from there we should use the correct primitives for cryptocurrency identity, custody, and transactions.  So I don't care about "cannot prevent double spending" - that's not what I'm expecting it to be good at.

    "Whether the AGPL core matters if our app only talks to a node over WebSocket has not been assessed"

This is a concern.  Say more about the author's intentions with regards to this.

## Holochain

I've been familiar with Holochain for many years now, and while I'm very impressed with their core architecture and abstractions, I also think that they went to far in defining a weird domain language that I have to learn before becoming productive... and they bolted too much runtime and stateful operation into it which I don't understand, and therefore don't trust in terms of building apps on top of it.  It feels like the toolchain is complex and has a lot of moving parts, and that gives me a lot of hesitancy.

## p2panda

This feels architecturally similar to Freenet: am I right about that?  It feels like both Freenet and p2panda have less of the overhead that I'm concerned about with Holochain.  I'm not really sure how to compare Freenet and p2panda side by side though... maybe I need to see a small reference implementation ported to the other so I can compare the two.

## KeyOxide

KeyOxide I feel misses the mark in terms of what I respected and valued most about KeyBase: the identity bootstrapping process.  Maybe the cryptographic claims is the core on which that process needs to be built though.  On the fence about whether this is useful at all.

## nostr

    "no User Keychain, no Device Constellation, and relays as a point of metadata visibility and censorship"

I feel like nostr was a response to the complexity of Holochain: they saw the kitchen sink approach and decided to come from the opposite end of the spectrum: build up from tiny bite sized protocols that could be adopted progressively, and which when selected individually might be useful to broader audiences.

The issue I have with this is that when you fail to implement things like "Peer Discovery", "Message Routing", "State Synchronization", or "Device Constellation", you end up leaving that to the developer to custom implement, and then we're right back in the paradigm we started with, where decentralized programming is hard, and so the economic incentives lead us back to centralized solutions "for now".  And "for now" becomes "forever".

So I'm not against nostr in principle, but:

1) If you're going to implement RAFT on top of nostr, why didn't you just implement RAFT in the first place?
2) If you're selecting relay nodes because they're simpler and have higher throughput, are you willing to just accept the custody challenges inherent in that?  How do we evolve from there when it's the fundamental primitive that everything is built on?

    "What it is: a protocol, not one implementation"

I would say it's a whole ecosystem of protocols and implementations.

## Ockam

    "its identity model is the best of the six for an identity root: an identifier derived from the first signed key change, a signed change history that rotates the primary key and revokes purpose keys, and attribute credentials checked by policy (ABAC). Those ideas are worth reimplementing"

    An aggregator says Ockam "dissolved in July 2026"

They were selling secure cryptographic channels to businesses.  I wonder what they learned about that market.  Does the market not value it?  Was their positioning off?

    Works over TCP, UDP, BLE and WebSocket, useful for NAT traversal, but assumes peers are online: a poor fit for mobile chat

Hmm... they are abstracting the encryption capsule from the transport, and they bragged about even being able to store messages in queue systems or at rest.  I don't think that what you're saying is accurate.

# Goals

I want to build an app that will handle chat and crypto wallet functionality.  It should:

    - have a KeyBase like device constellation, and the QR code scanning ergonomics that made that work
    - have (optional) KeyBase like attestation, linking other accounts in order to bolster your reputation
    - have a password manager
    - be able to contain other personal data: lets say MD notes files
    - strong custody model where users are asked for consent in data access, updating, and transference
    - a model for "apps" that is data-centric: apps are thin layers for interpretation and manipulation of data
    - local first data model: apps are built to interact with local data, state synchronization happens within the underlying core
    - a sister project, "Otto", implements business backend services on Cloudflare workers.  Can these services be ported to the new architecture?  Ideally we are designing with this in mind.  Primitives such as RPC and access controls would therefore be portable.
    - the core code for identity, data storage, communication, and state synchronization should be written in Rust.  UI bindings for Flutter and Vue.js are my first priorities.
