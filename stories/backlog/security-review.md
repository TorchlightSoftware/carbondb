# CarbonDB gets a security and privacy review after the prototype

Status: backlog

## Idea

Finish the initial prototype first, then carefully consider and revise its security and privacy.

## To review

- What publishing an identity reveals: its device public keys and proofs still show how many devices it has ([public identity proofs](public-identity-proofs.md)).
- What follows from having no server to order changes, such as two devices revoking each other concurrently, and defences such as a paper key or a hardware key required for device changes.
- Whether the paper key can stay outside p2panda, given that members above Pull need current key bundles.
- p2panda's group encryption, which is a custom protocol with no audit found.
- Dev and test mode, which keep keys in SQLite in plaintext, and the secure secret storage that production requires.
- Adversarial members, which the design does not yet war-game.
- Creating a channel or a service cannot be undone, and if a rewind refuses the op that created it, it stays, unattached. Who can create channels and services that outlive their attachment, and what that leaves behind.
- Loading compiled components from the cache directory runs them as native code, so whoever can write that directory can run code in the process ([docs/wasm-services.md](../../docs/wasm-services.md)).

## Other Questions

We now have a multi-threaded app, which compiles and caches WASM on the fly, and runs potentially untrusted service code.  This was meant to run on mobile phones, among other things.  What do we need to do to thoroughly test, so that there are no performance or security bugs that would compromise the environments we run in?
