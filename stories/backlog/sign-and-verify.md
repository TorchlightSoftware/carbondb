# An app can sign a message as an identity and verify one

Status: backlog

## Idea

Signing needs this device's private key, which only the system space holds, so signing is a `[@carbon]` call: a replicated service never sees a key and never calls `[@carbon]`. Verifying needs only public keys, which the identity space holds through its `[device]/origin` entries.

- `POST [@carbon]/sign` in an identity space signs a payload with this device's key, and answers the signature with the device's public key.
- `POST [@carbon]/verify` checks a signature against a device key, and that key against the identity's `[device]/origin` chain.

## To decide

- Whether verifying belongs in `[@carbon]` at all, or is a plain function an app runs, since it needs no private key.
- Whether verify reads the identity space on this device, or takes an exported identity, for an identity this device does not hold.
- Whether a signature names the identity or only the device, and how a verifier treats a device revoked after it signed.
- Whether a signature is a GET, since it changes nothing, or a POST kept in the op log, as a record of what this device signed.
- What the payload is: bytes, or JSON in a canonical form, and what the signature is over besides it, such as a purpose, to keep a signature for one use from being replayed in another.
- How this relates to public proofs, which are signed by a device and verified offline from the origin (see [the public identity proofs story](public-identity-proofs.md)).

## Related

- [Anyone can verify an identity's devices and public proofs](public-identity-proofs.md)
- [A person can revoke a lost device](revoke-device.md)
