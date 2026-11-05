# An app developer can install @tlsft/carbondb on any desktop platform

Status: backlog

## Idea

The build copies only the Linux `.so` into `packages/carbondb`, so the package runs on Linux alone. Native Node modules usually ship one prebuilt binary per platform and architecture as optional dependencies, and npm installs only the one that matches. napi-rs generates this layout and the CI matrix that builds it.

## To decide

- Which platforms come first: Linux x64 and arm64, macOS arm64, Windows x64.
- Whether CI builds and publishes them, or a release script does.
- How the mobile and browser builds relate: Flutter and the browser don't use the Node binding, so they are separate artifacts.
