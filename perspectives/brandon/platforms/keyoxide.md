# KeyOxide

Research date: 2026-10-26. See [README](README.md) for the vocabulary and the cross-platform matrix.

## Verdict for chat + wallet

KeyOxide is an identity-proof tool, not a network: it answers "is this key really @alice on that service?" through two-way linking, and nothing else in the stack. It is a candidate Public Claims add-on. Its newer profile format (ASP) has no key rotation or revocation, which rules it out as the root of a multi-device identity, and maintenance is fragile.

## Status

- **Maturity:** a small, working identity-proof tool. Repos on Codeberg ([org](https://codeberg.org/keyoxide)).
- **Releases:**
  - `doipjs` 2.1.0 (2024-12-03) is the last stable release; its `dev` branch is active, with an OpenPGP v6 / RFC 9580 PR merged 2026-09-28 by a non-founder maintainer ([PR 182](https://codeberg.org/keyoxide/doipjs/pulls/182)).
  - `keyoxide-web` 5.1.0 (2024-12-03), last commit 2025-09-24.
  - `aspe-server-rs` 0.2.1 (2024-02).
  - Spec last changed 2023-10; blog's last post 2023-10 ([blog](https://blog.keyoxide.org/)).
- **Licences:** `doipjs` Apache-2.0; `keyoxide-web` and `aspe-server-rs` AGPL-3.0; `aspe-dart` MIT.
- **Languages:** JavaScript, Rust, Dart.
- **Flutter:** the official mobile app is Flutter (AGPL) on Android, iOS and Linux. Very active (v2.4.5 shipped 2026-10-26) but effectively one developer ([repo](https://codeberg.org/Berker/keyoxide-flutter), [F-Droid metadata](https://gitlab.com/fdroid/fdroiddata/-/raw/master/metadata/org.keyoxide.keyoxide.yml)). A Dart ASPE client exists ([aspe-dart](https://codeberg.org/keyoxide/aspe-dart)).
- **Funding:** NLnet NGI grants in 2020 and 2022 ([nlnet 2](https://blog.keyoxide.org/nlnet-2/)), Keyoxide Mobile (NGI Assure, 2022-12 to 2024-07, [NLnet](https://nlnet.nl/project/Keyoxide-Mobile/)), OpenCollective. No current grant found.
- **Team health:** fragile. The founder worked on it full-time 2020–2024 and joined NLnet as a Rust developer in February 2024 ([NLnet](https://nlnet.nl/people/yarmo/)). Upkeep now looks volunteer and community-driven *(inferred from commit authors)*.

## Components

| Term | Provides? | How | Consequence for chat + wallet |
|---|---|---|---|
| Public & Secret Key | native | OpenPGP keys, or ASP: a signed JSON token (JWS) using EdDSA/Ed25519 or ES256/P-256 ([ASP spec](https://ariadne.id/related/ariadne-signature-profile-0/)) | Standard algorithms Dart can handle; Ed25519 reuses easily |
| User Keychain | partial | The app stores several PGP/ASP profiles, encrypted on device | Profiles only, not per-app or per-purpose keys |
| Self-Sovereign Identity | native | Identity is the key fingerprint (SHA-512 truncated, Base32), no registration | Good match; ASPE hosting optional and self-hostable |
| Device Constellation | none (ASP); partial (OpenPGP subkeys and revocation) | ASP defines no rotation or revocation; updates must be signed by the same key | Losing the device loses the ASP identity |
| Public Claims | native, core purpose | Two-way linking: the profile lists claim URIs and each account posts a proof back ([docs](https://docs.keyoxide.org/)) | Can verifiably link chat handles (Matrix, XMPP, fediverse) to a key |
| Private Claims | none | — | — |
| Contacts | partial | Contacts feature with lookup by fingerprint or email (WKD, keyserver, ASPE) | An identity lookup layer, not an address book you can message |
| Peer Discovery | none | — | — |
| Distributed Hash Table | none | Keyservers, WKD and ASPE are plain HTTP | — |
| Private Communication | none | Text encryption was a mobile-grant goal; not confirmed shipped | — |
| Blind Relay Nodes | none | — | — |
| Encrypted Storage as a Service | none | — | — |
| State Synchronization | none | — | — |
| Public Ledger | none | — | — |
| Currency / Wallet | none | A wallet address could be listed as a generic claim *(inferred)* | — |
| Reputation / QoS | partial *(inferred)* | Verified links to established accounts make fake identities costlier | Weak Sybil signal; no incentives |
| Public & Private Compute | — | Verification runs in the client, or on keyoxide.org plus doip proxy servers (`proxy3.keyoxide.org`) | Some web verifications need a proxy we would run ourselves |

## Architectural decisions that matter most

1. **Two-way public linking as the trust model.** Helps with "is this contact really @alice?"; does nothing for messaging itself.
2. **ASP chose plain JWS over OpenPGP.** Easy in Dart; no rotation or revocation, ruling it out as a multi-device identity root.
3. **Optional HTTP hosting (ASPE, WKD, keyservers).** No peer-to-peer layer; lookups depend on reachable HTTP hosts.
4. **Verification fetches live third-party pages.** Fragile when platforms change their APIs, as the many provider-fix commits show.

## Existing chat and wallet apps

None. KeyOxide is an identity-proof add-on, not a base to build on.

## Red flags and unknowns

- Single-maintainer Flutter app, and AGPL: forking it makes our app AGPL; `aspe-dart` (MIT) is the safe dependency.
- No stable `doipjs` release since 2024-12.
- No verifiable funding after 2024.
- ASP is still v0, and its spec contradicts itself on timing windows (60 seconds vs 60 minutes) and expiry.
