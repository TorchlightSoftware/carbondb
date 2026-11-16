# A manager can move a channel to a different parent

Status: backlog

## Idea

Parenthood is set when a channel is created, through the parent's ref to it and the manage list on the child's origin op ([docs/channels.md](../../docs/channels.md#reaching-channels-refs-and-handles)). Both are fixed today, so a team cannot move from one company to another, or out of a company on its own.

## To decide

- Which ops a move writes: a ref removed from the old parent, a ref added to the new one, and a change to the child's manage list.
- Whose manage rights it needs: the old parent's, the new parent's, the child's, or all three.
- What happens to members who reached the child only through the old parent, given `cascade: ['remove']`.
- How a move concurrent with other changes to either parent resolves on every replica.
