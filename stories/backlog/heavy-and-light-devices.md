# A person can host services on heavy devices and reach them from light ones

Status: backlog

## Idea

A person may assign some of their devices to a "heavy" group and others to a "light" group.

- **Heavy devices** do the hosting, such as a dedicated server in the living room or a paid relay on Cloudflare.
- **Light devices**, such as a phone or laptop, keep access control enabled but reach the services remotely.

`heavy` and `light` could be device tags in `[identity]`, alongside `manage`, read by the identity's assignment policy ([p2panda-sync.md](../p2panda-sync.md)), which places each service on suitable devices.

## Related

- [An identity can replace the default assignment policy](assignment-policy-override.md)
- A person grants remote access with roles without membership: remote access stays outside CarbonDB ([implementation.md](../implementation.md#remote-access-stays-outside-carbondb))
