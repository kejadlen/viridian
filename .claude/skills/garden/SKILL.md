---
name: garden
description: Use when working on viridian — after finishing a task or making a decision, record it as an entry in the ai garden at gardens/ai, and whenever adding or editing any node there
---

# Garden

From the repo root, build viri and run its guide, then follow it:

```sh
cargo build -p viri
./target/debug/viri __internal skill
```

The guide comes from viri's source, so it covers the commands this
checkout builds: the garden layout, the edit buffer, and driving the
editor from a script. Run every command below as
`./target/debug/viri --garden gardens/ai ...`.

## The ai garden

Work on viridian is tracked in viridian. The ai garden, tended by AI, is
its own jj repo at `gardens/ai`, gitignored inside viridian's checkout.
Every task finished and every decision made becomes an **entry**: a node
with a `parent` component for each node it concerns. There is no backlog;
record what happened, not what's planned.

## Record an entry

1. Pick the parents: every node the task or decision is about. Design
   decisions go on `soqlqxoq` (viridian-design), decisions about the
   build plan on `wopxuwkn` (first-slice), and anything broader on
   `qmplyuym` (viridian).
2. Record the entry with `viri new <parent-prefix>...`. Write the body
   as a `#` title, then what was decided or done and why, in a few
   sentences. Link the nodes it concerns.
3. If the entry makes a parent's body wrong, garden that parent with
   `viri edit` so it states the current plan. Entries keep the history;
   parents stay current.
4. Commit `viri.kdl` with `jj -R gardens/ai commit`, at the same time as
   the viridian commit for the work it records.

## Rules

- Design belongs in `soqlqxoq` (viridian-design). Implementation
  decisions never go there; they go in entries or a narrowly scoped
  node such as first-slice.
- Docs are nodes in the garden, never files in viridian's repo.
  `created` replaces a dated filename.
- Leave out components you don't need, including tags: relate nodes
  with links and parents, not tags.
