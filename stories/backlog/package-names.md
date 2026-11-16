# An app developer registers a service by its package or crate name

Status: backlog

## Idea

A channel names each of its services by a package registered on this device (see [a channel names its services' packages](../packages.md#a-channel-names-its-services-packages)). A package is then known by a name of its own, unique across packages, rather than by the name a call uses in some channel. Today an app registers a service under a device-wide name, which overrides the canonical name in its `service.toml`.

## To decide

- How a package is identified across devices: a name unique across packages, as npm scopes give, or its version hash.
- Whether the app registers a package by its name and CarbonDB finds its component, or the app imports the component and registers it, as today.
- Whether the device-wide name passed to `services.add` gives way to the name `POST [channel]/services` gives each service, binding it to a package. Running two copies of one package in an app then needs no special case: two names, in one channel or two, bound to the same package.
