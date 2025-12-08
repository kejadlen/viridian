# viridian

## brainstorming

- text content stores as markdown files so they can be tracked and versioned in git
  - `change id - slug.md`
- change ids a la jujutsu
- metadata in sqlite db
- viri cli
  - completions for change ids, etc
  - `viri edit ...`
    - makes a proxy representation of the note as a tempfile
    - use lsp (`viri lsp`?) and filetype for completions, etc
    - ref: https://matklad.github.io/2024/12/13/majjit-lsp.html
- first class support for notes as journal entries on other notes
  - notes are gardened but entries allow for history
- entity types a la ECS systems
  - restaurants, recipes, etc.
  - contacts?
- assets
- static site generation
- tags
  - relationships
    - recommended by
    - source
- archiving
- semantic clustering
- multiple nodes
  - so private / public / work are separate nodes but can be interlinked?
  - this part needs a lot of thought development here
  - each node is a "garden" maybe?
