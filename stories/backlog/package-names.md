# An app developer registers a service by its package or crate name

Status: backlog

## Idea

A package's `service.toml` holds its canonical name (see [packages](../packages.md)). An app registers a service under the name it takes locally in the space, which overrides the canonical one. Mapping canonical npm package and crate names onto services would let an app register a service by its package name, with CarbonDB finding the package's `service.toml` from it.

## To decide

- Whether the app registers a package by its name and CarbonDB finds its `service.toml`, or the package exports where its `service.toml` is. [packages](../packages.md) says CarbonDB never finds npm packages; the app imports and registers them.
- How a canonical name relates to the local name the app registers, when an app runs two copies of one package.
