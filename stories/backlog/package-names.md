# An app developer registers a service by its package or crate name

Status: backlog

## Idea

The destination is that a device registers packages, and a space's origin binds each service name it runs to one of them (see [a space's origin naming its packages](../packages.md#a-spaces-origin-names-its-packages)). A package is then known by a name of its own, unique across packages, rather than by the name a call uses in some space. Today an app registers a service under a device-wide name, which overrides the canonical name in its `service.toml`.

## To decide

- How a package is identified across devices: a name unique across packages, as npm scopes give, or its version hash.
- Whether the app registers a package by its name and CarbonDB finds its `service.toml`, or the package exports where its `service.toml` is, as today. [packages](../packages.md) says CarbonDB never finds npm packages; the app imports and registers them.
- Whether the local name moves from `services.add` to space creation, which binds each name to a package. Running two copies of one package in an app then needs no special case: two spaces, or two names in one space, bound to the same package.
