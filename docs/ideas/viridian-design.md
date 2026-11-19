# Viridian design

## Scope

This document records the current goals and design for viridian, a
personal knowledge system with a `viri` CLI and a native iOS app. It
covers the goal, the node and component model, storage and sync, gardens
and the cross-garden link rule, the leak check, editing, queries,
publishing, and the first buildable slice.

It doesn't cover the query language, the public authoring web UI's
feature set, POSSE syndication, assets, archiving, semantic clustering,
the work garden's own infrastructure, or a web or macOS app, which may
come later. Those are deferred, and the
unsettled ones are listed under [open questions](#open-questions).

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
`restaurant` with `url`, `address`, and `status`. Each one has a schema,
declared up front, and viri-core validates every component against its
schema on every write. Schemas validate components, not whole nodes, so
a node can carry several components or none. Long-form text belongs in
the body, not in a field, so a recipe's instructions go in its body.

The model follows the entity component system (ECS) pattern, with a
node as the entity. The processes that act on components, such as the
indexer, the site build, and the leak check, are its systems.

A node may also be an **entry**: an immutable, timestamped node attached
to a parent node by the parent's change id. Entries record the history
of a subject, while the parent stays freely editable.

Node ids are jujutsu-style random change ids, displayed by their
shortest unique prefix. Body files are named `<change-id> <slug>.viri`,
and links reference only the change id, so a slug can change without
breaking links. Importing the archive migrates each legacy 12-digit
timestamp id to a new change id and rewrites the links that use it. The
old ids are not kept as aliases.

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
that are rarely online together, builds the static site, and hosts the
public authoring web UI.

The following table shows what each device holds and how it's used:

| Device | Gardens held | Interface |
|---|---|---|
| Personal laptop | Personal, public | `viri` CLI, `viri edit`, and jj on garden working copies |
| iPhone | Personal, public | Native iOS app |
| Server | Personal, public | Sync relay, site build, and authoring web UI |
| Work laptop | Work only | A browser for editing public through the authoring web UI |

The iOS app is a SwiftUI shell over viri-core, a Rust crate exposed
through UniFFI. viri-core embeds the git and Automerge libraries, so the
CLI and the app share one implementation of sync, schemas, and
validation.

## Gardens and scope

A **garden** is a set of nodes with a single scope that governs both
which devices sync it and who can see it. The following table shows the
three gardens:

| Garden | Copies held by | Served to |
|---|---|---|
| Personal | Personal laptop, iPhone, server | Your devices, over Tailscale |
| Public | Personal laptop, iPhone, server | The internet, as a static site |
| Work | The work machine | The work machine only |

The personal garden's git remote and Automerge relay are reachable only
over the tailnet, so the network enforces personal's scope. The public
garden's git repo may live on an internet-facing forge.

The work laptop can't join the tailnet or run custom tooling. It edits
public through the authoring web UI, which is the only internet-facing
write surface.

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

- A public node's body or metadata references a non-public change id.
- A garden's root document lists a node from another garden.
- The site build reads anything outside the public garden.

A private change id in public content is an error even though the id
itself is opaque, because the link text beside it is free-form.

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

When a fetch produces a body conflict, viri marks the node as
conflicted. You resolve it with `viri edit` or with jj on the laptop.

## Schema changes

Schema migrations are manual. Before running one, you confirm that every
device has synced; viri doesn't track devices or enforce this. If a
device writes against the old schema after a migration, cleaning up
that data is up to you.

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

The first buildable slice runs on the laptop only, against the personal
garden, with no sync. It includes:

- viri-core and the `viri` CLI
- Declared schemas and validation
- Automerge metadata stored locally
- Bodies in a git repo
- The SQLite index
- `viri edit`, with validation
- `viri query`
- An import of `archive/notes`

This slice tests structure and query against real data before any sync,
iOS, LSP, site, or web UI work. With only one garden, the leak check
waits until public exists.

## Open questions

- Where schemas live. They are assumed to be documents in the metadata
  store, so a new component syncs to every device without a rebuild.
- The query language exposed by `viri query`.
- The public authoring web UI's feature set and login mechanism.
- How entry immutability holds up against hand edits to bodies in git,
  and whether `viri check` flags edited entries.
- How the iOS app presents sync conflicts.
- Which git library viri-core uses. `git2` supports push; whether
  gitoxide does is unverified.
- How renames work, given that a slug change renames the body file in
  git.
- The work garden's infrastructure, since the work machine can't run
  custom tooling.
- POSSE syndication: platforms, which nodes syndicate, and whether
  replies flow back.
