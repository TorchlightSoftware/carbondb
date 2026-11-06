# An app developer's npm service runs at the version npm installed

Status: backlog

## Why

Every op records its service's version, and a replica refuses an op made for a version it doesn't run, so two devices never run different code under one version and drift. Today an npm service's version is a hash the JS runtime computes over its handlers' source, its `init` and the migrations it exports (`packages/carbondb/src/services.js`). That misses every module a handler imports: change a helper and the version stays the same, so replicas can drift exactly as versioning is meant to prevent.

## Already agreed

From [packages](../packages.md):

- An npm package's version is the integrity npm records for it in the lockfile. A host may pass any non-empty string instead.
- CarbonDB offers a utility that looks up an installed package's integrity in the lockfile.
- Workspace-linked packages have no integrity and are not accommodated.

## Idea

`services.add` takes the version from the app, by default through the lookup utility, and the source hash goes away. The core already stamps the version on each op and refuses ops made for another, so nothing changes there.

## To decide

- Whether `services.add` takes the version as an argument, or the package name and looks it up itself.
- What `[chat]`, a workspace package in this repository, passes as its version, since it has no integrity.

## Testing

- The lookup utility reads a fixture lockfile and returns the integrity of a named package, and fails clearly for a package that isn't there or is workspace-linked.
- A service registered with an integrity stamps it on its ops; reopening with a different integrity refuses the ops left without a result, as `refuses_an_op_made_for_another_version_of_its_service` does for Rust.
