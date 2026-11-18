# A service calls other services only by the names its channel gives them

Status: backlog

## Idea

The `call` effect takes the id of the space to post to, beside the call's key and input (`crates/core/src/calls/dispatch.rs`, `call_effect`). Without an id, `[name]` resolves through the caller's channel; with one, the call goes straight to that space. Today any service may name an id, so a service that learns another's id can post to it from outside its channel, and the target's own checks, such as manage, are its only guard.

One caller needs an id today: `[@carbon]`, which creates spaces and sets them up before any channel names them, as when it founds an identity or creates a channel. Every other service should reach services only by the names its channel gives them.

## To decide

- Whether the core keeps the id only for `[@carbon]`, refusing it from any other service.
- What a refused call answers: a 403, recorded like any effect's output.
