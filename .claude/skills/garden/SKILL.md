---
name: garden
description: Use when working on viridian — after finishing a task or making a decision, record it as an entry in the ai garden at gardens/ai, and whenever adding or editing any node there
---

# Garden

Work on viridian is tracked in viridian. The ai garden, tended by AI, is
its own jj repo at `gardens/ai`, gitignored inside viridian's checkout.
Every task finished and every decision made becomes an **entry**: a node
with a `parent` component for each node it concerns. There is no backlog;
record what happened, not what's planned.

## Layout

- Bodies are `gardens/ai/<id> <slug>.viri`, markdown.
- Metadata for every node is one `node` block in `gardens/ai/viri.kdl`.
- Ids are 8 characters: 4 random bytes in jj's reverse-hex alphabet.

```kdl
node "<id>" {
    slug "<kebab-slug>"
    created "<UTC timestamp>"
    parent "<parent id>"
    parent "<another parent id>"
}
```

Values are quoted strings, so the file is valid KDL. `slug` and
`created` are required. Every other child is a component, and `parent`
can repeat. Leave out components you don't need, including
tags: relate nodes with links and parents, not tags.

## Record an entry

1. Pick the parents: every node the task or decision is about. Design
   decisions go on `soqlqxoq` (viridian-design), decisions about the
   build plan on `wopxuwkn` (first-slice), and anything broader on
   `qmplyuym` (viridian).
2. Record the entry with `viri`, from the repo root:

   ```sh
   cargo build -p viri
   ./target/debug/viri --garden gardens/ai new <parent-prefix>...
   ```

   Fill in the `slug` and write the body in the editor: a `#` title, then
   what was decided or done and why, in a few sentences. Link other nodes
   as `[text](viri:<id>)`. `viri new` commits the body below `@` and
   appends the `node` block to `viri.kdl`.

   To record an entry without the editor (as when driving `viri` from a
   script), set `EDITOR` to a small program that rewrites the buffer file
   it's given.
3. If the entry makes a parent's body wrong, garden that parent so it
   states the current plan. Entries keep the history; parents stay
   current. `viri edit <prefix>` edits a node; committing the `viri.kdl`
   change in `@` is still yours to do.
4. Commit the working copy with `jj -R gardens/ai commit`, at the
   same time as the viridian commit for the work it records.

## Rules

- Design belongs in `soqlqxoq` (viridian-design). Implementation
  decisions never go there; they go in entries or a narrowly scoped
  node such as first-slice.
- Docs are nodes in the garden, never files in viridian's repo.
  `created` replaces a dated filename.
- Never rename a body file by hand without updating its slug in
  `viri.kdl`.
