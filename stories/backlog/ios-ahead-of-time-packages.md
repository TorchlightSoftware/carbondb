# An iOS user runs the app's shipped services at native speed

Status: backlog

## Idea

iOS forbids JIT, so WASM packages there run in an interpreter such as wasmi, and a JS service compiled to WASM runs in QuickJS inside that interpreter, which is slow. Packages shipped with the app are known when it is built, so they can be compiled ahead of time into it. Only packages installed at runtime would then be interpreted.

## To decide

- Which tool compiles them ahead of time, such as wasmtime's precompiled modules, and whether App Store review accepts it.
- Whether a compiled package keeps the hash of its original `.wasm` bytes, so peers still see the same version.
- Whether Android does the same, for battery rather than policy.
