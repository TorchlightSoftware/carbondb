# A service calls other services only by the names its channel gives them

Status: backlog

## Idea

The `call` effect takes the id of the space to post to, beside the call's key and input (`crates/core/src/calls/dispatch.rs`, `call_effect`). Without an id, `[name]` resolves through the caller's channel; with one, the call goes straight to that space. Today any service may name an id, so a service that learns another's id can post to it from outside its channel, and the target's own checks, such as manage, are its only guard.

Two callers need an id today: `[@carbon]`, founding an identity's channels before any of them names the others, and `[channel]`, writing the origin of a channel it has just created. Every other service should reach services only by the names its channel gives them.

## To decide

- Whether the core keeps the id only for `[@carbon]` and for `[channel]` posting into a space its own run created, refusing it from any other service.
- Whether `[channel]` instead writes a new channel's origin through a step of the `create` effect, so no service ever names an id.
- What a refused call answers: a 403, recorded like any effect's output.
