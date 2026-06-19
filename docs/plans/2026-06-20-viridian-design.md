# Viridian design

## Scope

This document records the core architecture for viridian: a personal
knowledge system with a `viri` CLI and a web UI, served from a single
always-on personal server. It covers the storage model, the node/entry
distinction, the garden/scope model, the cross-garden link rule, and how
devices reach the system.

It does **not** cover features still in early brainstorming: entity types
and components (the ECS idea), tags and relationship semantics, static
site generation specifics, assets, archiving, semantic clustering, the
work garden's own infrastructure, or the `viri edit` LSP projection beyond
a sketch. Those build on the foundation here and are deferred.

This is a design record for the author, who is also the implementer. It
assumes familiarity with markdown, SQLite, jujutsu's change ids, and
Tailscale.

## Architecture: one authoritative server, many clients

Viridian is server-centric. A single always-on personal server holds the
authoritative store; phones and laptops are clients that reach it over the
network. There is no peer-to-peer replication and no multi-master sync.

This reverses an earlier peer-sync direction. Earlier plans gave each
machine a co-equal authoritative store and tried to reconcile them, which
is a distributed-systems project (CRDTs, op logs, replicated databases).
Adding phone access made co-equal per-machine stores untenable — a phone
will not run a CLI, a database replica, and an LSP editor — and a single
authoritative server removes the reconciliation problem entirely.

The server holds the **personal** and **public** gardens together: prose
as `<change-id> <slug>.viri` files on disk, and metadata — identity,
entry/parent links, components, tags, relationships — in SQLite. Both live
on the same machine, so there is no cross-store sync to manage. The
SQLite database is authoritative for metadata and is backed up alongside
the files.

## Access follows the visibility order at the network layer

Each garden's scope determines how it is served, so the visibility order
becomes largely a network property rather than application logic:

- **Personal** is the `viri` API and web UI, served **only on the
  Tailscale tailnet**. Your personal laptop and phone reach it; the public
  internet cannot route to it. Tailscale supplies device identity, so
  private access needs no separate authentication system.
- **Public** is served two ways from the same server-side garden: a
  read-only static site on the **public internet**, and an authenticated
  authoring web UI, also on the internet, for editing public content from a
  device that is not on the tailnet.
- **Work** stays isolated on the work machine, never on the server or the
  phone. It references public, nothing more.

The work laptop drives the authenticated authoring path. It cannot join the
tailnet and cannot run custom tooling, but it has a browser and can reach
the internet, so it edits public through the authenticated web UI. This
adds a single internet-facing write surface for public, where before the
internet saw only the read-only site. Because the author is a single user,
the login stays minimal — single-user OIDC or one password over HTTPS, with
no user-management system. Personal has no internet write surface and no
route from the internet at all.

## Clients

Each device gets the interface its capabilities and trust level allow:

- **Personal laptop** runs the rich loop — the `viri` CLI, the LSP, and the
  majjit-style editing projection — against the server over the tailnet,
  most likely through a local working copy it checks out and pushes back,
  since the LSP needs files on disk. It can author both personal and public.
- **Phone** uses the web UI over Tailscale for personal (reading and quick
  capture) and the static site over the internet for public. It keeps no
  local store.
- **Work laptop** uses the authenticated public authoring web UI over the
  internet. Editing public from work is browser-only: no CLI and no LSP, so
  the rich editing loop stays on the personal laptop. The work laptop never
  reaches personal.

Storage is uniform across every garden: server-side prose files plus a
co-located SQLite database. The web UI and the static site are different
front ends onto that one storage model, not different models.

## Nodes and entries

Everything in viridian is a **node**, stored as a `<change-id> <slug>.viri`
file. There is one kind of node. By default a node is mutable and
evergreen: you garden it, rewriting freely, and its history is not
significant.

A node may also be an **entry**. An entry attaches to a parent node and
records a timestamped moment in that subject's history. Being an entry is a
property a node takes on, not a separate kind of file — in the ECS sense, a
component. Entries are immutable and append-only: you add a new entry
rather than editing an existing one. An entry is still a full node, so it
can carry its own links, types, and even its own entries.

The first daily workflow is capturing an entry against a subject: open a
node (a person, a project, a topic) and append a timestamped entry. The
entry is its own node with its own change id, referencing its parent by the
parent's change id.

Change ids follow jujutsu's model: globally unique and stable. In the
filename the change id leads and a single space separates it from the slug,
so a parser splits on the first space and the slug may contain its own
hyphens. The slug is human sugar that may drift as you rename; links always
reference the stable change id, never the slug. The `.viri` file holds
prose only — markdown body, no frontmatter — because metadata is
authoritative in SQLite, not in the file. The `.viri` extension drives
editor and LSP filetype association.

## Gardens and scope

A **garden** is a set of nodes with a single **scope** that governs both
where it is served and who can see it — one concept, not two:

| Garden | Authoritative location | Served to |
|---|---|---|
| personal | the server | your devices, over Tailscale |
| public | the server | the internet, as a static site |
| work | the work machine | the work machine only |

## The cross-garden link rule

Scopes form a partial order by availability. References flow only toward
more-available gardens:

```
   personal     work        most private, incomparable
        \       /
         public             most available; the sink
```

A node may link to gardens at least as available as its own. `personal →
public` and `work → public` resolve; `public →` anything private and
`personal ↔ work` do not. A link resolves only when the consumer can reach
the target garden; otherwise it degrades to plain text. The network
topology enforces this: a public-internet visitor cannot route to the
personal garden, so a leaking link is unreachable by construction.

## Open questions

- **Offline capture.** Server-centric means online-centric: off the tailnet
  or with the server down, you cannot capture to personal. Decide whether
  offline capture matters; if so, clients need a local outbox that syncs
  when reachable.
- **Laptop client model.** Pure API client versus a local working copy the
  laptop checks out and pushes back, and how the LSP editing loop fits.
- **Out-of-band edits.** The reconcile rule when prose files on the server
  are changed outside `viri`.
- **The `viri edit` projection.** How the LSP-backed editable view merges
  metadata and prose and writes changes back.
- **Public authoring UI scope.** How much of the editing model the
  browser-only public authoring UI exposes, given it has no LSP, and the
  exact login mechanism (single-user OIDC versus a password).
- **Deferred features.** Entity types and components, tags and relationship
  semantics, static site generation, assets, archiving, and semantic
  clustering.
