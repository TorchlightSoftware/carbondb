# CarbonDB architecture components

Goal: decide upon core CarbonDB architecture for chat and crypto wallet.

One file per architectural component, named with the terms of the [Vision for Decentralized Web](https://github.com/bitmage/vision-for-decentralized-web) wiki; related wiki terms are folded into the component they serve. Each file records how the candidate networks fill that component, what is agreed, and what is still open. Per-network evaluations, the feature matrix and sources (as of 2026-10-26) live in [platforms/](../platforms/README.md). No candidate covers every component, so the architecture will be a composition of networks, libraries and our own build.

A component is on the critical path when the goal can't be reached without deciding it.

| Component | Critical path | Depends on |
|---|---|---|
| [Identity](identity.md) | yes | — |
| [Device Constellation](device-constellation.md) | yes | Identity, State Synchronization |
| [Contacts](contacts.md) | yes | — |
| [Private Communication](private-communication.md) | yes | Identity, Device Constellation, Contacts |
| [Blind Relay Nodes](blind-relay-nodes.md) | yes | — |
| [State Synchronization](state-synchronization.md) | yes | — |
| [Encrypted Storage as a Service](encrypted-storage-as-a-service.md) | no | — |
| [Crypto Wallet](crypto-wallet.md) | yes | Identity |
| [Client Platform](client-platform.md) | yes | — |
| [Service Calls](service-calls.md) | no | Identity, Device Constellation |
