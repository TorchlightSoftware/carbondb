# Someone outside a channel can message one of its services

Status: backlog

## Why

A customer placing an order, or a stranger asking to join, should be able to message a service without being able to read it. Today the only way into a service is to replicate it, and replicating gives read.

## What we know

- p2panda's access levels are ordered, and a greater level contains every lower one, so write includes read: a device that can post to a space can read and decrypt all of it.
- p2panda attaches application-defined conditions to access (`Access<C>`), not yet examined as a way to narrow write.
- Pull is the lowest level and every level contains it. A pull-only member is left out of the encryption context and gets no keys: it carries ciphertext it cannot read, the blind relay the README describes.

## Idea

A sender who is not a replication peer messages a service over HTTP, through the side-effect binding; p2panda access stays for replication peers. The service receiving the message records it as an op of its own, so every replica applies it the same way.

## To decide

- Who serves the HTTP endpoint for a service: a member's always-on device, a relay, or the service's chosen host.
- How the receiving service authenticates the sender and limits what it may post.
- How the sender learns the outcome, given that only delivery is guaranteed between services.
