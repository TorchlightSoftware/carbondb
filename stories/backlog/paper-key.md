# A person gets a paper key when they create an identity

Status: backlog

## Idea

[Story 01](../01-initialize-identity.md) returns a paper key once from `POST [@carbon]/identity` and never stores it. The build returns none, because recovery is not designed. A paper key is an offline device: in Keybase's device chain it is a full-powered device key, and it is the usual way back in when every device is lost.

Already agreed (see [implementation](../implementation.md)): the paper key is an identity log entry only, and stays out of the p2panda group until recovery is designed.

## To decide

- Whether to generate and return it now as an identity log entry, or wait until recovery is designed.
- Whether it is a full-powered device, as in Keybase (see [device constellation](../../perspectives/brandon/components/device-constellation.md)).
- Whether a paper key must authorize device changes, such as revoking a device (see [revoke a device](revoke-device.md) and the [security review](security-review.md)).
