# A person can host services on heavy devices and reach them from light ones

Status: backlog

## Idea

A person may assign some of their devices to a "heavy" group and others to a "light" group.

- **Heavy devices** do the hosting, such as a dedicated server in the living room or a paid relay on Cloudflare.
- **Light devices**, such as a phone or laptop, keep access control enabled but reach the services remotely.

`heavy` and `light` could be device tags on `POST [device]/origin`, alongside `manage`.

## Related

- [A person grants remote access with roles without membership](../README.md#backlog)
