# viri

viri tends a garden of nodes. This guide comes from the `viri` binary
that printed it, so it matches that build's commands.

## Gardens and nodes

A garden is a directory holding a `viri.kdl` metadata file and a jj repo
for node bodies. The user config, `$XDG_CONFIG_HOME/viri/config.kdl` or
`~/.config/viri/config.kdl`, lists gardens as `garden "<path>"` lines
and can name a `default "<name>"`. A garden's name is the `garden` line
in its `viri.kdl`.

`ls`, `show`, and `edit` cover every configured garden plus the nearest
ancestor directory with a `viri.kdl`. `new` uses that ancestor, else the
default, else the only known garden. `--garden <name|path>` picks the
garden for `new` and narrows the others to it. A value with a `/` is a
path, so write `./notes`, not `notes`.

Every node has an 8-character jj-style change id, and any unique prefix
of it names the node. A node's body is markdown in
`<id> <slug>.viri`. Its metadata is one `node` block in `viri.kdl`:

```kdl
node "<id>" {
    slug "<kebab-slug>"
    created "<UTC timestamp>"
    tags "<tag>"
    parent "<parent id>"
}
```

Values are quoted strings, so the file is valid KDL. `slug` and
`created` are required. Every other child is a component, and `tags` and
`parent` can repeat. Link to another node from a body with
`[text](viri:<id>)`.

## Commands

| Command | Effect |
|---|---|
| `viri new [<parent>...]` | Opens the editor on a new node, with a `parent` component for each id prefix given |
| `viri edit <prefix>` | Opens the editor on an existing node |
| `viri ls` | Lists each node's shortest unique id prefix, slug, and tags, writing a slug two gardens share as `<garden>/<slug>` |
| `viri show <prefix>` | Prints a node in the edit buffer's format |

`viri new` prints the new node's short id and slug.

## The edit buffer

`viri new` and `viri edit` open a buffer with the metadata as
frontmatter, followed by the body:

```
---
id: kxrvwzpm
slug: sesame-noodles
created: 2026-11-21T16:02:11Z
tags:
  - recipe
parents:
  - qmplyuym
---
# Sesame noodles
```

`viri new` leaves `slug:` empty for you to fill. Don't change `id`.

On save, `viri` validates the buffer. If it's valid, `viri` commits the
body below the working-copy commit `@`, as `viri: new <slug>` or `viri:
edit <slug>`, and writes the `node` block to `viri.kdl`. A changed slug
renames the body file in a commit of its own. `viri.kdl` stays
uncommitted in `@`; commit it yourself with jj.

If the buffer isn't valid, the editor reopens with the errors as
comments. Quitting without fixing them keeps the buffer in
`.viri/recovery/` and changes nothing.

## Editing without a terminal

`viri` runs `$VISUAL`, then `$EDITOR`, then `vi`, passing the buffer's
path. To drive it from a script, set both `VISUAL` and `EDITOR` to a
program that rewrites that file. `VISUAL` wins, so setting only `EDITOR`
hangs on whatever editor `VISUAL` names. For example, to record a node
whose body is in `body.md`:

```sh
cat > fill.sh <<'EOF'
#!/bin/sh
set -eu
sed "s/^slug:\$/slug: $SLUG/" "$1" > "$1.tmp"
cat "$BODY" >> "$1.tmp"
mv "$1.tmp" "$1"
EOF
chmod +x fill.sh
VISUAL=./fill.sh EDITOR=./fill.sh SLUG=my-slug BODY=body.md \
    viri new <parent-prefix>
```

To edit a node, write the whole buffer: start from `viri show
<prefix>`, change it, and have the script copy it over `$1`.

## Hand edits

Bodies can also be edited directly and committed with jj. Never rename
a body file by hand; change its slug through `viri edit`, which renames
the file.
