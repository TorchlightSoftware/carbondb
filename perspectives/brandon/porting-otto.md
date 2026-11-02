# Porting Otto onto CarbonDB

Otto is a private service platform of ours, on Cloudflare Workers. It is what CarbonDB's first service runtime is meant to grow towards, so its specifics are kept here rather than in `stories/`, which describe CarbonDB without depending on it.

## The runtime

- Replicated mode lives in candle-db, with QueryRunner transforming queries as it already does for tenant scoping. Queries port unchanged through it.
- candle-db schema defaults (`$defaultFn`, `$onUpdateFn`) derive ids and timestamps from the op in replicated mode. Otto's service code has 44 direct calls to replace.
- candle-db's audit log is replaced by the space's op log.
- candle-db `validate` functions are self-contained row predicates, the shape a receiver-side validator needs.
- `query-runner.ts` imports `drizzle-orm/d1` at line 8 and defines `createDrizzleClient` at line 16, and the rest of the runner is driver-agnostic. Drizzle's sqlite-proxy driver sends (sql, params, method) to any executor, and Otto's planned D1-shaped shim for host-node is a seam a CarbonDB host could reuse instead.
- Building host-node in Otto first helps every host, including CarbonDB. It concerns hosting Otto services, not [chat].

## The handler context

- `connect` becomes a service-key call over iroh, `files` becomes p2panda-blobs, and `request` and `env` need small shims.
- What `env` carries with no Cloudflare bindings or deploy-time secret store is open. The Otto runtime builds it, as Cloudflare does today, and secrets cannot come from a replicated space.
- Otto handlers that check current state, such as `getYear` then `notFound`, become local UX, not enforcement. Business rules run on every replica.

## Roles

- Otto's [identity] service holds roles. It is a different service from CarbonDB's [identity], which holds only an identity's origin, and the two never run in the same space.
- Otto's roles are global sets of route-key features, assigned to identities per tenant, and a role can span several services.
- With no server to issue a JWT, the verified op author replaces it. The Otto runtime builds a claims object compatible with Otto's `JWTPayload` from the op.
- Roles are kept current by triggers on `POST [space]/members`. A handler cannot read another space deterministically, so a role check finds its roles in the same space.

## Service errors

- `@tlsft/service-error` lives in the otto repo, beside `@tlsft/service-key`. CarbonDB is public, so it must be published as a public package before CarbonDB can depend on it.
- Otto's errors are in `core/candle-env/src/error.js`, with factories such as `notFound` (404, `NOT_FOUND`). MicroQL's are in `src/common/error-catalog.js`, a catalog of templates such as `unknown-resource` (404). Both are an Error carrying `status` and `code`.
- They differ in three ways:
  - **Codes:** Otto uses generic upper-case codes per status. MicroQL uses specific kebab-case codes per cause.
  - **Details:** Otto keeps them in a `details` field. MicroQL spreads them onto the error, with `baseMessage` and `hint`.
  - **Normalising:** Otto normalises any thrown value with a numeric `status`.

## HTTP

Otto's `Service.fetch(request, ctx)` in `core/candle-service/src/service.ts` is a request-to-response function, and Cloudflare or host-node does the listening. A CarbonDB HTTP adapter has the same shape.

## Offline cutoffs

Otto's `audit.ts` had a 90-day drop window measured by the receiver's clock. Clocks differ between replicas, so on CarbonDB that window would make them disagree about which ops to refuse.
