# Oplog examples

Each block is a space with its ops indented under it, in the order they run. `[@carbon]` is local: it runs only on this device, in the context of that space. The calls listed after it in the same block are its sub-calls: they travel on its log row, and the space's other members replay them, never the `[@carbon]` op itself. Every other op on a replicated service reaches the space's other members as itself.

## Alice creates an identity and a company

The identity space holds only what may be published. Everything private, including the company's ref, hangs from the personal space.

```
system
  POST [@carbon]/identity {slug:'alice', device:'Pixel9', services:['contacts']}   # generates the key pair

identity:alice
  POST [space]/origin {kind:'identity', services:['identity','device'], manage:[self]}
  POST [identity]/origin {device:'asdf942'}                     # its hash is the identity's id
  POST [device]/origin {pubkey:'asdf942'}
  POST [space]/ref {space:personal:alice, kind:'personal'}

personal:alice
  POST [space]/origin {kind:'personal', services:['device','contacts'], manage:[identity:alice]}
  POST [device]/ref {pubkey:'asdf942', name:'Pixel9', tags:['manage']}
  POST [@carbon]/space {kind:'company', name:'FooCorp', services:['identity','chat','books'], manage:[identity:alice]}
  POST [space]/ref {space:company:foo-corp, kind:'company', name:'FooCorp'}

company:foo-corp
  POST [space]/origin {kind:'company', name:'FooCorp', services:['identity','chat','books'], manage:[identity:alice]}
```

## Alice creates the teams

Each team is created from FooCorp, so it runs under FooCorp's rules. FooCorp accepts the team's ref first; if it refuses, the team is not attached to FooCorp.

```
company:foo-corp
  POST [@carbon]/space {kind:'team', name:'Sales', services:['crm'], manage:[company:foo-corp], cascade:['remove','delete']}
  POST [space]/ref {space:team:sales, kind:'team', name:'Sales', cascade:['remove','delete']}

team:sales
  POST [space]/origin {kind:'team', name:'Sales', services:['crm'], manage:[company:foo-corp]}

company:foo-corp
  POST [@carbon]/space {kind:'team', name:'Marketing', services:['website','sms','automation'], manage:[company:foo-corp], cascade:['remove','delete']}
  POST [space]/ref {space:team:marketing, kind:'team', name:'Marketing', cascade:['remove','delete']}

team:marketing
  POST [space]/origin {kind:'team', name:'Marketing', services:['website','sms','automation'], manage:[company:foo-corp]}

company:foo-corp
  POST [@carbon]/space {kind:'team', name:'Service', services:['worklog'], manage:[company:foo-corp], cascade:['remove','delete']}
  POST [space]/ref {space:team:service, kind:'team', name:'Service', cascade:['remove','delete']}

team:service
  POST [space]/origin {kind:'team', name:'Service', services:['worklog'], manage:[company:foo-corp]}
```

## Alice adds members

Being added to a team does not add someone to the company, so each person has a `members` op in both. Tom is shown; Sally joins FooCorp and Marketing, and Hank and Forest join FooCorp and Service, the same way.

```
company:foo-corp
  POST [@carbon]/members {contact:tom}                          # returns an invite code; any medium may carry it

team:sales
  POST [@carbon]/members {contact:tom}                          # returns an invite code
```

## Tom starts from zero and joins

Tom's personal space holds a ref to FooCorp only. He reaches Sales by looking in FooCorp, because his access comes through FooCorp, so nothing about Sales goes in his personal space. He joins Sales from FooCorp's handle, and FooCorp already holds its ref.

```
system
  POST [@carbon]/identity {slug:'tom', device:'TomPhone', services:[]}   # generates the key pair

identity:tom
  POST [space]/origin {kind:'identity', services:['identity','device'], manage:[self]}
  POST [identity]/origin {device:'zxcv305'}
  POST [device]/origin {pubkey:'zxcv305'}
  POST [space]/ref {space:personal:tom, kind:'personal'}

personal:tom
  POST [space]/origin {kind:'personal', services:['device'], manage:[identity:tom]}
  POST [device]/ref {pubkey:'zxcv305', name:'TomPhone', tags:['manage']}
  POST [@carbon]/join {code:'foocorp-code'}
  POST [space]/ref {space:company:foo-corp, kind:'company', name:'FooCorp'}

company:foo-corp
  POST [space]/members {identity:tom}                           # written by Pixel9 once it sees Tom join
  POST [@carbon]/join {code:'sales-code'}                       # FooCorp already holds this ref

team:sales
  POST [space]/members {identity:tom}                           # written by Pixel9 once it sees Tom join
```

## Tom adds a second device

```
# on TomPhone, showing a code
identity:tom
  POST [@carbon]/devices                                        # answers 'tom-code'

# on the new device, after scanning it
system
  POST [@carbon]/identity {code:'tom-code', device:'TomTablet'} # generates the key pair and hands TomPhone its public key

# on TomPhone, once the new device answers (how the exchange runs is open, see story 02)
identity:tom
  POST [device]/origin {pubkey:'qwer117'}

personal:tom
  POST [device]/ref {pubkey:'qwer117', name:'TomTablet', tags:[]}

# on the new device, once it has synced identity:tom
identity:tom
  GET [space]/refs                                              # personal:tom

personal:tom
  GET [space]/refs                                              # FooCorp

company:foo-corp
  GET [space]/refs                                              # Sales, Marketing, Service

team:sales
  GET [@carbon]/status                                          # what Tom may do here depends on his access
```

## Notes

- A `[space]/ref` only points; it grants nothing. Reaching a space needs both the refs to exist and access to the space, and what is possible there depends on that access. Which teams to show someone is up to the services running in the space.
- `POST [device]/origin` is what authorizes a key to act for an identity, and it carries only the public key, because the identity space may be published. A device's name and tags go in a `[device]/ref` in the personal space, and its private key stays in `system`.
- `[identity]` in an identity space is CarbonDB's own service for the identity's origin. The `identity` in FooCorp's services is the app's own role service. They are different services, which needs [a space's origin naming its packages](packages.md#a-spaces-origin-names-its-packages), not built yet.
- `manage` is CarbonDB's own permission. CarbonDB maps it onto p2panda's Manage, which p2panda grants only to individual devices.
- `manage:[self]` on an identity space means the identity manages itself, since its id does not exist until its origin ops are written.
- `manage:[]` means no one may delete the space or add members; the space dissolves when all members leave.
- `cascade` lives on the ref. `remove` means losing access to FooCorp removes access to Sales; `delete` means deleting FooCorp deletes Sales, given manage rights there. Without `delete`, Sales stays.
- `manage` and `cascade` are set when a space is created. Moving a space to a different manager is undefined.
