# An identity can replace the default assignment policy

Status: backlog

## Idea

An identity's assignment policy reads the tags on its devices and the tags on the services in a space, and assigns which devices hold that space at pull, read, write or manage. The CarbonDB core provides a default policy.

An identity may later supply its own policy in place of the default. We do not yet know the use cases; testing the default policy will show them.

## To decide

- What an identity needs from its own policy that the default policy cannot give.
- How an identity supplies one, and where it is kept.

## Related

- [A person can host services on heavy devices and reach them from light ones](heavy-and-light-devices.md)
