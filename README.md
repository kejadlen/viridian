# viridian

Viridian is a personal knowledge system that gives notes structure you
can query. Every note is a node: a markdown body plus typed components,
such as tags, parents, or a restaurant's address. Bodies live as files
in a jj repo, so they're versioned and editable by hand. `viri` is the
command-line tool for working with a garden of nodes.

Viridian is early. The first slice runs on one laptop with no sync. It
can create, edit, list, and show nodes. Queries, imports, sync, the
static site, and the iOS app come later.

## Build

Building needs a stable Rust toolchain. From the repository root:

```sh
cargo build -p viri
```

The binary lands at `target/debug/viri`.

## Use

A garden is a directory that holds a `viri.kdl` metadata file and a jj
repo for the node bodies. To start one:

```sh
mkdir notes && cd notes
jj git init
printf 'garden "personal"\n' > viri.kdl
```

To use several gardens from anywhere, list them in
`$XDG_CONFIG_HOME/viri/config.kdl`, or in `~/.config/viri/config.kdl`
when `XDG_CONFIG_HOME` is unset. macOS uses the same paths:

```kdl
garden "~/notes/personal"
garden "~/notes/public"
default "personal"
```

Each `garden` line is a garden's directory, either absolute or starting
with `~/`. A garden's name comes from the `garden` line in its own
`viri.kdl`.

`ls`, `show`, and `edit` cover every configured garden plus the one
you're in, and id prefixes are unique across all of them. `new` puts
the node in the garden you're in, else in the `default` garden, else in
the only garden `viri` knows. With several gardens and none of those
to go on, it asks which one, or fails when it has no terminal to ask
on. Pass `--garden <name>` or `--garden <path>` to choose the garden
for `new`, or to narrow the other commands to one garden. A value with
a `/` in it, such as `./notes`, is a path.

`viri` has the following commands:

| Command | Effect |
|---|---|
| `viri new [<parent>...]` | Opens the editor on a new node, with a `parent` component for each id prefix given |
| `viri edit <prefix>` | Opens the editor on an existing node |
| `viri ls` | Lists each node's short id, slug, and tags, writing a slug two gardens share as `<garden>/<slug>` |
| `viri show <prefix>` | Prints a node in the editor's format |

Ids are 8-character jj-style change ids. Any unique prefix names a node.

The editor comes from `$VISUAL` or `$EDITOR`. It opens a buffer with
the node's metadata as YAML frontmatter, followed by the body:

```
---
id: kxrvwzpm
slug: sesame-noodles
created: 2026-11-21T16:02:11Z
tags:
  - recipe
---
# Sesame noodles
```

On save, `viri` validates the frontmatter. If it's valid, `viri`
commits the body below the working-copy commit `@` and writes the
metadata to `viri.kdl`, which you commit yourself. If it isn't valid,
the editor reopens with the errors as comments. Quitting without fixing
them keeps the buffer in `.viri/recovery/` and changes nothing.

Bodies are stored as `<id> <slug>.viri`. You can also edit them
directly and commit with jj. Link to another node with
`[text](viri:<id>)`.

## Develop

The `justfile` has the following recipes:

| Recipe | Effect |
|---|---|
| `just fmt` | Formats the workspace |
| `just clippy` | Lints with warnings as errors |
| `just test` | Runs the unit, property, and CLI tests |
| `just coverage` | Fails if library line coverage is below 100% |
| `just mutants` | Runs cargo-mutants |
| `just all` | Runs fmt, clippy, test, and coverage |

The workspace has two crates. `viri-core` is the library every client
shares: ids, schemas, metadata, bodies, and the garden that joins them.
`viri` is the CLI, plus the edit buffer and the editor loop.

`archive/` holds the notes and journal pages that predate viridian,
waiting to be imported.

## Design

The design and the record of work on viridian are nodes in the ai
garden, a separate jj repo at `gardens/ai`. That directory is ignored
here and has no remote yet, so it exists only on the author's laptop.
Start from the `viridian` node, `qmplyuym`.
