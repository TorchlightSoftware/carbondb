# Service Calls

Critical path: no. Depends on: [Identity](identity.md), [Device Constellation](device-constellation.md).

Not a wiki term: how business services (an API with resources, verbs, inputs, outputs and access rules) run on CarbonDB, so that identity, access control and calls carry over from a hosted backend to devices and peers.

## What a service is

The definitions come from two public projects and also a private one (Otto) which has a more extensive library of services.

- [`@tlsft/service-key`](https://www.npmjs.com/package/@tlsft/service-key) (the naming grammar)
- [MicroQL](https://github.com/TorchlightSoftware/microql) (the service contract, specified in its [service guide](https://github.com/TorchlightSoftware/microql/blob/main/docs/service-guide.md)).

Roughly, a service is a set of endpoints which encapsulates a set of DOMAIN principles and enforces data integrity and business rules.  It can be a REST API, gRPC, SOAP etc.  Service calls take named parameters, and their inputs and outputs are described by zod schemas.

- **A service call is named by a service key**, `[METHOD] [service]/resource`, e.g. `POST [billing]/invoices` or `[helloworld.Greeter]/SayHello`. The key names *what* it is (service, resource, verb), never *where* it lives or what transport protocol to use (this is different from a URI).
- **Mapping a service to an address and transport is subjective** - Address and transport lookup is up to the caller's lookup table. The method is optional, so REST and RPC calls can both use service key grammar.
- **Access controls can be defined for service keys** The service key provides a canonical name on which access controls can be performed.  So each service key can also be thought of as a distinct "feature".  Wildcard description is also possible with (`* [*]/**`).

## Context

- In context of CarbonDB service calls could be made upon a service controlled by a single user.
- If {Frank} is in your address book, you could call services offered by Frank, subject to the access controls defined and the proof of identity that you present.
- This would allow portability of traditional REST and server side semantics over to a decentralized environment.

## Agreed

## Open questions

- Not clear how this would interplay with the concepts of decentralized write access.
- Not clear whether access control semantics map cleanly.
- Not clear how process and storage replication semantics would map.
- Do decentralized data and service-controlled data operate differently, or can they be unified?
- When {frank} has several devices, which one answers a call to his services?
