# Viridian design

## Scope

This document records the current goals and design for viridian, a
personal knowledge system with a `viri` CLI and a native iOS app. It
covers the goal, the node and component model, schemas, storage and
sync, gardens and the cross-garden link rule, the leak check, editing,
queries, publishing, and the first buildable slice.

It doesn't cover the query language, POSSE syndication, assets,
archiving, semantic clustering, or a web or macOS app, which may come
later. Those are deferred, and the unsettled ones are listed under
[open questions](#open-questions).

This is a design record for the author, who is also the implementer. It
assumes familiarity with markdown, git, jujutsu's change ids,
[Automerge][automerge], SQLite, and Tailscale.

[automerge]: https://automerge.org/

## Goal

Viridian exists to give notes structure that can be queried. The notes
in `archive/notes` already hold structured data by hand. For example, a
restaurant note carries `url` and `address` frontmatter, `#restaurant
#seattle` tags, and dated sections about visits. A separate "seattle
restaurants" note lists restaurants by hand and has drifted out of step
with the per-restaurant notes. Viridian should replace a list like that
with a query.

Capture from anywhere, including a phone, matters but ranks below
structure. Daily journaling is not a goal, and the journal pages in
`archive/journal` don't need a model of their own.

## Nodes, components, and entries

Everything in viridian is a **node**. Every node has a fixed core: a
change id, a garden, a slug, and a creation time. A node also carries
zero or more components.

A **component** is a named piece of data attached to a node. The
**body** is the one unstructured component: the node's markdown text. A
node has at most one body, and a node without one, such as a bookmark,
is valid. Most imported notes carry a body and nothing else.

Every other component is a named group of short, typed fields, such as
`restaurant` with `url`, `address`, and `status`. Each one has a
[schema](#schemas), and viri-core validates every component against its
schema on every write. Schemas validate components, not whole nodes, so
a node can carry several components or none. Long-form text belongs in
the body, not in a field, so a recipe's instructions go in its body.

The model follows the entity component system (ECS) pattern, with a
node as the entity. The processes that act on components, such as the
indexer, the site build, and the leak check, are its systems.

A node may also be an **entry**: a timestamped node attached to a
parent node by the parent's change id. Entries record the history of a
subject. They're editable like any other node, and git keeps the
history of their bodies.

Node ids are jujutsu-style random change ids, displayed by their
shortest unique prefix. Importing the archive migrates each legacy
12-digit timestamp id to a new change id and rewrites the links that use
it. The old ids are not kept as aliases.

Body files are named `<change-id> <slug>.viri`. Metadata owns the slug,
and the slug in a filename is cosmetic: viri finds a body by its
change-id prefix, splitting the filename on the first space. Whenever
viri writes to a garden, it renames any file whose slug is stale, in a
commit of its own. Two body files with the same change id are a `viri
check` error. Links reference only the change id, so renames never break
them.

## Schemas

Schemas are declared up front, compiled into viri-core, and shared by
every garden. Adding or changing a component means rebuilding and
redeploying viri on every device. Because every device and the public
relay carry the full set, schema names and fields are not secret.

Devices can run builds with different schema sets, such as an iOS app a
release behind the laptop. A build that receives a component it doesn't
know preserves it untouched on every save and shows it read-only, as raw
fields. Only a build that knows the schema can edit the component.

Schema migrations are manual. Before running one, you confirm that every
device has synced; viri doesn't track devices or enforce this. If a
device writes against the old schema after a migration, cleaning up
that data is up to you.

## Storage

The body is stored apart from everything else. This document calls a
node's core fields and its other components its **metadata**. Bodies and
metadata live in separate stores with separate sync engines. The
following table shows where each kind of data lives:

| Data | Store | Sync |
|---|---|---|
| Bodies | One git repo of `.viri` files per garden | git push and fetch |
| Metadata (core fields, other components, typed relationships) | Automerge, one document per node, plus one root document per garden that lists its nodes | Automerge sync |
| Query index | SQLite, one per device | Not synced; rebuilt from the other two |

Metadata owns a node's identity and existence. A node exists when its
metadata document exists, and deleting a node writes a tombstone to its
metadata. A device that holds a body file without the matching metadata
hides the file until the metadata arrives. A device that holds metadata
without the body yet shows the node with its body pending.

Because each node has its own metadata document, a garden's root
document is the access boundary. A process given only the public root
never receives personal metadata. The cost is that Automerge can't
change several documents atomically, so an operation such as creating an
entry and linking it to its parent spans two documents.

Keeping bodies in git means concurrent body edits from two devices
surface as git conflicts instead of merging character by character.

## Devices and sync

Every device holds full copies of the gardens it can see and syncs them
whenever it reaches a peer. An always-on server is one of those peers,
not an authority. It relays git and Automerge changes between devices
that are rarely online together, and it builds the static site.

The following table shows what each device holds and how it's used:

| Device | Gardens held | Interface |
|---|---|---|
| Personal laptop | Personal, public | `viri` CLI, `viri edit`, and jj on garden working copies |
| iPhone | Personal, public | Native iOS app |
| Work laptop | Public, work | `viri` CLI, `viri edit`, and jj on garden working copies |
| Server | Personal, public | Sync relays and the site build |

The server runs two Automerge relays. The tailnet relay serves personal
and public to devices on the tailnet. The public relay is a separate
process that loads only the public garden, so it can't leak personal
metadata it never loaded. It's reachable from the internet so the work
laptop, which can't join the tailnet, can sync public. Each device
authenticates to it over TLS with its own token, which `viri device
revoke` invalidates. The public relay and the public git forge are the
only internet-facing write surfaces.

The iOS app is a SwiftUI shell over viri-core, a Rust crate exposed
through UniFFI. viri-core embeds the git and Automerge libraries, so
every client shares one implementation of sync, schemas, and validation.

viri-core uses [`git2`][git2] (libgit2) for git. As of November 2026,
[gitoxide's status page][gix-status] lists push, merge orchestration,
and SSH without an external `ssh` binary as unimplemented, and the iOS
app needs all three.

[git2]: https://crates.io/crates/git2
[gix-status]: https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md

## Gardens and scope

A **garden** is a set of nodes with a single scope that governs both
which devices sync it and who can see it. The following table shows the
three gardens:

| Garden | Copies held by | Served to |
|---|---|---|
| Personal | Personal laptop, iPhone, server | Your devices, over Tailscale |
| Public | Personal laptop, iPhone, work laptop, server | The internet, as a static site |
| Work | The work laptop | The work laptop only |

The personal garden's git remote and Automerge relay are reachable only
over the tailnet, so the network enforces personal's scope. The public
garden's git repo may live on an internet-facing forge, and its
metadata syncs to the work laptop through the public relay.

The work garden has no remote. It never leaves the work laptop, which
also means it has no backup anywhere else.

References flow only toward more available gardens:

```
   personal     work        most private, incomparable
        \       /
         public             most available; the sink
```

`personal → public` and `work → public` links resolve. Links from public
to anything private, and links between personal and work, are not
allowed.

## The leak check

The leak check enforces the link rule and the garden boundaries. It
fails when any of the following is true:

- A public node's body or metadata references a change id that isn't a
  known public node.
- A garden's root document lists a node from another garden.
- The site build reads anything outside the public garden.

A private change id in public content is an error even though the id
itself is opaque, because the link text beside it is free-form. An id
the device can't resolve also fails, because the check can't prove it's
public. On the work laptop, which holds no personal nodes, that rule
catches a personal id pasted into public.

viri-core implements one checker, which runs in these places:

- On every `viri edit` save
- As `viri check`, run by hand
- As a local pre-push hook on the public garden's repo
- In the static site build, which fails closed

Pushes to the public forge are not checked on the server. A leak in a
hand commit that skips the pre-push hook is readable on the forge until
the site build catches it. A CI check on the forge may close that gap
later.

## Editing

`viri edit <change-id>` writes the node to a tempfile as YAML
frontmatter (the node's other components) followed by the body. An LSP
flags schema errors while you edit.

On save, viri-core validates the frontmatter against the schemas. If it
validates, viri-core applies the metadata as Automerge changes, writes
the body to the garden's working copy, and commits it with a message
built from the change id and slug. If it doesn't validate, viri writes
nothing and reopens the editor with the errors as YAML comments at the
top. Quitting without fixing the errors keeps the buffer in a recovery
file and changes nothing.

Metadata is edited only through `viri edit` or the iOS app. A body may
also be edited directly in a garden's working copy and committed by hand
with jj. Hand edits bypass save-time validation, which is why the leak
check also runs before pushes and in the site build.

When a fetch produces a body conflict, viri-core completes the merge
anyway, committing the conflict markers into the body, and flags the
node as conflicted in the index. A repo is never left mid-merge, so
other nodes keep syncing. You can resolve the conflict on any device:
the iOS app offers keep mine, keep theirs, or edit, and a laptop uses
`viri edit` or jj.

## Links

Links inside a body reference a node by change id only, for example
`[harissa](viri:kxrvwzpm)`. Typed relationships, such as "recommended
by" and "source," are component fields in metadata, validated by schema.

## Queries

Automerge stores documents but can't query across them, so each device
keeps a SQLite index built from two sources: the metadata documents,
and links parsed out of bodies. The index is disposable. It's never
edited, synced, or backed up, and losing it means rebuilding it.

## Publishing

The server builds a static site from the public garden alone. A
published node's URL is `/<short-change-id>/<slug>`. The site resolves
it by change id, so a stale slug redirects to the current one. URLs from
the old blog posts in the archive are not preserved.

## First slice

The first buildable slice runs on the personal laptop only, against the
personal garden, with no sync. It includes:

- viri-core and the `viri` CLI
- Compiled schemas and validation
- Automerge metadata stored locally
- Bodies in a git repo
- The SQLite index
- `viri edit`, with validation
- `viri query`
- An import of `archive/notes`

This slice tests structure and query against real data before any sync,
iOS, LSP, or site work. With only one garden, the leak check waits until
public exists.

## Open questions

- The query language exposed by `viri query`. The first slice includes
  `viri query`, so this has to be settled before that slice is done.
- Whether the work garden needs a backup, given that it has no remote.
- Whether to move to gitoxide once it supports push and SSH.
- POSSE syndication: platforms, which nodes syndicate, and whether
  replies flow back. This waits until the static site exists.
