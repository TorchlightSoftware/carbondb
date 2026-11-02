# CarbonDB gets a security and privacy review after the prototype

Status: backlog

## Idea

Finish the initial prototype first, then carefully consider and revise its security and privacy.

## To review

- What publishing an identity reveals: the identity space holds only its origin, device public keys and proofs, which still shows how many devices it has.
- What follows from having no server to order changes, such as two devices revoking each other concurrently, and defences such as a paper key or a hardware key required for device changes.
- Whether the paper key can stay outside the p2panda group, given that members above Pull need current key bundles.
- p2panda's group encryption, which is a custom protocol with no audit found.
- Dev and test mode, which keep keys in SQLite in plaintext, and the secure secret storage that production requires.
- Adversarial members, which the design does not yet war-game.
- Creating a space cannot be undone, and if a rewind rejects a parent's ref the space stays, unattached. Who can create spaces that outlive their attachment, and what that leaves behind.
