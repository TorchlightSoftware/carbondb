# Oplog examples

Each block is one service's op log, named by its channel and the name the channel gives it, such as `personal:alice [channel]`, with its ops in the order they run. Every service has a log of its own, in its own space. `[@carbon]` runs only on this device and its ops go in the `system` log, which never replicates; what it does in a channel it does by calls made as effects, and each such call is an op in the log of the service it reaches, which replicates as itself. Effects an op makes are noted after `->`. Identities in manage lists, such as `identity:alice`, stand for the identity's id.

## Alice creates an identity and a company

The identity's own channel holds only its `[identity]` and its one ref, to the personal channel, and is sealed once founded. Everything else, including the company's ref, hangs from the personal channel.

```
system
  POST [@carbon]/identity {slug:'alice', device:'Pixel9', services:['contacts']}   # generates the key pair; -> create identity:alice's [channel], then each call below in identity:alice and personal:alice

identity:alice [channel]
  POST [channel]/origin {kind:'identity', name:'alice', manage:[identity:alice]}
  POST [channel]/services {name:'identity'}                                  # -> create the [identity] service
  POST [channel]/channel {kind:'personal', name:'alice', slug:'alice'}       # -> create personal:alice's [channel]; -> call its POST [channel]/origin; then the ref here
  POST [channel]/seal                                                        # last: every later POST here is refused

identity:alice [identity]
  POST [identity]/origin {device:'asdf942', name:'Pixel9', tags:['manage']}  # the identity's id is derived from this call's device key

personal:alice [channel]
  POST [channel]/origin {kind:'personal', name:'alice', manage:[identity:alice]}
  POST [channel]/services {name:'contacts'}                                  # -> create the [contacts] service
  POST [channel]/channel {kind:'company', name:'FooCorp'}                    # -> create company:foo-corp's [channel]; -> call its POST [channel]/origin; then the ref here

company:foo-corp [channel]
  POST [channel]/origin {kind:'company', name:'FooCorp', manage:[identity:alice]}
  POST [channel]/services {name:'identity', package:'roles'}                 # the app's role service, named [identity] in this channel
  POST [channel]/services {name:'chat'}
  POST [channel]/services {name:'books'}
```

## Alice creates the teams

Each team is created from FooCorp's handle, so the POST runs in FooCorp's `[channel]` and needs `manage` there; if it is refused, nothing is created. Its creation is an effect, so if a rewind later refuses the op at its place, the team stays, unattached.

```
company:foo-corp [channel]
  POST [channel]/channel {kind:'team', name:'Sales', manage:[company:foo-corp], cascade:['remove','delete']}        # -> create; -> call its origin; then the ref here
  POST [channel]/channel {kind:'team', name:'Marketing', manage:[company:foo-corp], cascade:['remove','delete']}
  POST [channel]/channel {kind:'team', name:'Service', manage:[company:foo-corp], cascade:['remove','delete']}

team:sales [channel]
  POST [channel]/origin {kind:'team', name:'Sales', manage:[company:foo-corp]}
  POST [channel]/services {name:'crm'}

team:marketing [channel]
  POST [channel]/origin {kind:'team', name:'Marketing', manage:[company:foo-corp]}
  POST [channel]/services {name:'website'}
  POST [channel]/services {name:'sms'}
  POST [channel]/services {name:'automation'}

team:service [channel]
  POST [channel]/origin {kind:'team', name:'Service', manage:[company:foo-corp]}
  POST [channel]/services {name:'worklog'}
```

## Alice adds members

Being added to a team does not add someone to the company, so each person is invited to both. Tom is shown; Sally joins FooCorp and Marketing, and Hank and Forest join FooCorp and Service, the same way. Single-node, these fail as unreachable.

```
company:foo-corp [channel]
  POST [channel]/members {contact:tom, access:'write'}     # -> invite: an invitation key joins as a member; answers {invitation, code}; any medium may carry the code

team:sales [channel]
  POST [channel]/members {contact:tom, access:'write'}     # -> invite
```

## Tom starts from zero and joins

Tom's personal channel holds a ref to FooCorp only. He reaches Sales by looking in FooCorp, because his access comes through FooCorp, so nothing about Sales goes in his personal channel.

```
system
  POST [@carbon]/identity {slug:'tom', device:'TomPhone'}   # generates the key pair

identity:tom [channel]
  POST [channel]/origin {kind:'identity', name:'tom', manage:[identity:tom]}
  POST [channel]/services {name:'identity'}
  POST [channel]/channel {kind:'personal', name:'tom', slug:'tom'}
  POST [channel]/seal

identity:tom [identity]
  POST [identity]/origin {device:'zxcv305', name:'TomPhone', tags:['manage']}

personal:tom [channel]
  POST [channel]/origin {kind:'personal', name:'tom', manage:[identity:tom]}
  POST [channel]/join {code:'foocorp-code'}                 # -> join: answers FooCorp's channel, kind and name; then the ref here

company:foo-corp [channel]
  # Not yet defined: the op that records Tom as a member once he accepts, and when a manager's device replaces the invitation key with his devices
```

Not yet defined: how Tom joins Sales. `POST [channel]/join` needs `manage` on the channel it runs on, so Tom cannot join from FooCorp's handle, which already points to Sales; joining from his personal channel would put a ref to Sales there.

## Tom adds a second device

Needs peers; single-node, `POST [identity]/devices` is unreachable.

```
# on TomPhone, showing a code
identity:tom [identity]
  POST [identity]/devices {name:'TomTablet', tags:[]}        # -> enroll: answers {code:'tom-code'}

# on the new device, after scanning it
system
  POST [@carbon]/identity {slug:'tom', code:'tom-code', device:'TomTablet'}   # generates the key pair and hands TomPhone its public key

# on TomPhone, once the new device answers (how the exchange runs is open, see story 02)
identity:tom [identity]
  # Not yet defined: the op that records {pubkey:'qwer117', name:'TomTablet', tags:[]}. Scanning the other way, POST [identity]/devices {code, name, tags} records it in one op

# on the new device, once it has synced identity:tom
identity:tom [channel]
  GET [channel]/refs                                         # personal:tom

personal:tom [channel]
  GET [channel]/refs                                         # FooCorp

company:foo-corp [channel]
  GET [channel]/refs                                         # Sales, Marketing, Service

system
  GET [@carbon]/status {channel:<team:sales>}                # what Tom may do there depends on his access
```

Enrolling adds TomTablet to every service of Tom's own and personal channels. Adding it to FooCorp's and Sales's services needs a device with `manage` there online ([member devices](backlog/member-devices.md)).

## Notes

- A `[channel]` ref only points; it grants nothing. Reaching a channel needs both the refs to exist and access to the channel, and what is possible there depends on that access. Which teams to show someone is up to the services running in the channel.
- A ref to a new channel is written by the `POST [channel]/channel` op itself, not by an op of its own.
- `POST [identity]/origin` and `POST [identity]/devices` record a device's public key with its name and tags, all in `[identity]`, which only the person's own devices share. Its private key stays in `system`.
- `[identity]` in an identity's own channel is CarbonDB's own service for the identity's origin and devices. The `[identity]` FooCorp names is the app's own role service, registered on the device as `roles`. Names resolve per channel, so they never collide; a package name is still device-wide, so an app cannot register a package called `identity`.
- `manage` is CarbonDB's own permission, per channel. CarbonDB maps it onto p2panda's Manage, which p2panda grants only to individual devices.
- `manage:[identity:alice]` on Alice's own channel lists the identity itself; its id is known before its origin is written, since it is derived from the first device's key.
- `manage:[company:foo-corp]` stands for FooCorp's managers: a manage entry naming a channel is recorded in the child as the identities it expands to. Not built.
- `manage:[]` means no one may change the channel: add services or members, or delete it.
- `cascade` lives on the ref. `remove` means losing access to FooCorp removes access to Sales; `delete` means deleting FooCorp deletes Sales, given manage rights there. Without `delete`, Sales stays.
- `manage` and `cascade` are set when a channel is created; `POST [channel]/manage` adds managers. Moving a channel to a different parent is undefined ([backlog](backlog/reassign-space-parent.md)).
