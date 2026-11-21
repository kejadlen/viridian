//! Metadata for a garden: one hand-friendly `viri.kdl` file.
//!
//! The file is a `garden "<name>"` line followed by one `node` block per
//! node. Values are quoted strings, so the file is valid KDL and the `kdl`
//! crate parses it. Unknown components are preserved untouched; a build that
//! doesn't know a schema can still round-trip it.

use std::collections::BTreeSet;
use std::str::FromStr;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use jiff::Timestamp;
use kdl::KdlDocument;
use kdl::KdlError;
use kdl::KdlNode;
use miette::Diagnostic;
use thiserror::Error;

use crate::id::NodeId;
use crate::schema::NodeMeta;

/// The metadata store: a parsed `viri.kdl`, mutable in memory, written back
/// in a canonical hand-friendly style.
#[derive(Debug)]
pub struct Meta {
    path: Utf8PathBuf,
    name: String,
    blocks: Vec<Block>,
}

/// One `node` block: the typed metadata plus any components this build
/// doesn't know, preserved for the write.
#[derive(Debug)]
struct Block {
    meta: NodeMeta,
    extra: Vec<KdlNode>,
}

impl Meta {
    pub fn open(path: &Utf8Path) -> Result<Meta, MetaError> {
        let text = fs_err::read_to_string(path).map_err(|err| MetaError::Io {
            path: path.to_owned(),
            source: err,
        })?;
        Meta::parse(path.to_owned(), &text)
    }

    fn parse(path: Utf8PathBuf, text: &str) -> Result<Meta, MetaError> {
        let doc: KdlDocument = text.parse().map_err(|err| MetaError::Parse {
            path: path.clone(),
            source: err,
        })?;
        let invalid = |message: String| MetaError::Invalid {
            path: path.clone(),
            message,
        };

        let mut nodes = doc.nodes().iter();
        let name = match nodes.next() {
            Some(node) if node.name().value() == "garden" => {
                only_string_arg(node).map_err(invalid)?
            }
            _ => {
                return Err(invalid(
                    "expected the file to start with a `garden \"<name>\"` line".to_owned(),
                ));
            }
        };

        let mut blocks = Vec::new();
        let mut ids = BTreeSet::new();
        for node in nodes {
            if node.name().value() != "node" {
                return Err(invalid(format!(
                    "unexpected top-level node `{}`, expected `node`",
                    node.name().value()
                )));
            }
            let id: NodeId = only_string_arg(node)
                .map_err(invalid)?
                .parse()
                .map_err(|_| invalid(format!("`{}` is not a node id", node.name().value())))?;
            if !ids.insert(id) {
                return Err(invalid(format!("node {id} appears twice")));
            }
            blocks.push(parse_block(id, node).map_err(invalid)?);
        }
        Ok(Meta { path, name, blocks })
    }

    pub fn garden_name(&self) -> &str {
        &self.name
    }

    pub fn ids(&self) -> BTreeSet<NodeId> {
        self.blocks.iter().map(|b| b.meta.id).collect()
    }

    pub fn get(&self, id: &NodeId) -> Option<&NodeMeta> {
        self.blocks
            .iter()
            .find(|b| b.meta.id == *id)
            .map(|b| &b.meta)
    }

    /// Inserts or replaces a node's metadata, appending new nodes at the end
    /// so file order stays creation order.
    pub fn upsert(&mut self, meta: NodeMeta) {
        match self.blocks.iter_mut().find(|b| b.meta.id == meta.id) {
            Some(block) => block.meta = meta,
            None => self.blocks.push(Block {
                meta,
                extra: vec![],
            }),
        }
    }

    pub fn nodes(&self) -> impl Iterator<Item = &NodeMeta> {
        self.blocks.iter().map(|b| &b.meta)
    }

    /// Writes the file through a tempfile and rename, so a crash mid-write
    /// can't truncate the metadata.
    pub fn write(&self) -> Result<(), MetaError> {
        let text = self.to_text();
        let tmp = self.path.with_extension("kdl.tmp");
        fs_err::write(&tmp, text).map_err(|err| MetaError::Io {
            path: tmp.clone(),
            source: err,
        })?;
        fs_err::rename(&tmp, &self.path).map_err(|err| MetaError::Io {
            path: self.path.clone(),
            source: err,
        })?;
        Ok(())
    }

    fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("garden \"{}\"\n", self.name));
        for block in &self.blocks {
            out.push_str("\nnode \"");
            out.push_str(&block.meta.id.to_string());
            out.push_str("\" {\n");
            if let Some(slug) = block.meta.slug.as_deref() {
                out.push_str(&format!("    slug \"{slug}\"\n"));
            }
            if let Some(created) = block.meta.created {
                out.push_str(&format!("    created \"{created}\"\n"));
            }
            for tag in &block.meta.tags {
                out.push_str(&format!("    tags \"{tag}\"\n"));
            }
            for parent in &block.meta.parents {
                out.push_str(&format!("    parent \"{parent}\"\n"));
            }
            for extra in &block.extra {
                write_node(&mut out, extra, 1);
            }
            out.push_str("}\n");
        }
        out
    }
}

/// Writes an unknown component in the file's canonical style, normalizing
/// whatever formatting it was parsed with. Comments inside it are lost;
/// the component's data is not.
fn write_node(out: &mut String, node: &KdlNode, depth: usize) {
    let indent = "    ".repeat(depth);
    out.push_str(&indent);
    out.push_str(node.name().value());
    for entry in node.entries() {
        match entry.name() {
            Some(name) => out.push_str(&format!(" {name}={}", kdl_value(entry.value()))),
            None => out.push_str(&format!(" {}", kdl_value(entry.value()))),
        }
    }
    if let Some(children) = node.children() {
        out.push_str(" {\n");
        for child in children.nodes() {
            write_node(out, child, depth.saturating_add(1));
        }
        out.push_str(&indent);
        out.push_str("}\n");
    } else {
        out.push('\n');
    }
}

/// A KDL literal for a value; `KdlValue`'s own `Display` prints strings
/// bare, which would not round-trip.
fn kdl_value(value: &kdl::KdlValue) -> String {
    match value {
        kdl::KdlValue::String(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            format!("\"{escaped}\"")
        }
        other => other.to_string(),
    }
}

fn parse_block(id: NodeId, node: &KdlNode) -> Result<Block, String> {
    let mut slug = None;
    let mut created = None;
    let mut tags = Vec::new();
    let mut parents = Vec::new();
    let mut extra = Vec::new();

    let children = node
        .children()
        .ok_or_else(|| format!("node {id} has no `{{ }}` block"))?;
    for child in children.nodes() {
        let name = child.name().value();
        let value = || only_string_arg(child);
        match name {
            "slug" => {
                if slug.is_some() {
                    return Err(format!("node {id} has two `slug` lines"));
                }
                slug = Some(value()?);
            }
            "created" => {
                if created.is_some() {
                    return Err(format!("node {id} has two `created` lines"));
                }
                let raw = value()?;
                created = Some(
                    Timestamp::from_str(&raw)
                        .map_err(|_| format!("node {id}: `{raw}` is not an RFC 3339 timestamp"))?,
                );
            }
            "tags" => tags.push(value()?),
            "parent" => {
                let raw = value()?;
                let parent: NodeId = raw
                    .parse()
                    .map_err(|_| format!("node {id}: `{raw}` is not a node id"))?;
                parents.push(parent);
            }
            _ => extra.push(child.clone()),
        }
    }
    let slug = slug.ok_or_else(|| format!("node {id} has no `slug` line"))?;
    let created = created.ok_or_else(|| format!("node {id} has no `created` line"))?;
    Ok(Block {
        meta: NodeMeta {
            id,
            slug: Some(slug),
            created: Some(created),
            tags,
            parents,
        },
        extra,
    })
}

fn only_string_arg(node: &KdlNode) -> Result<String, String> {
    let args: Vec<_> = node
        .entries()
        .iter()
        .filter(|e| e.name().is_none())
        .collect();
    match args.as_slice() {
        [arg] => {
            arg.value().as_string().map(str::to_owned).ok_or_else(|| {
                format!("`{}` expects a quoted string argument", node.name().value())
            })
        }
        _ => Err(format!(
            "`{}` expects exactly one argument",
            node.name().value()
        )),
    }
}

#[derive(Error, Diagnostic, Debug)]
pub enum MetaError {
    #[error("failed to read {}", .path)]
    #[diagnostic(code(viri_core::meta::io))]
    Io {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse {}", .path)]
    #[diagnostic(code(viri_core::meta::parse))]
    Parse {
        path: Utf8PathBuf,
        #[source]
        source: KdlError,
    },

    #[error("{}: {message}", .path)]
    #[diagnostic(code(viri_core::meta::invalid))]
    Invalid { path: Utf8PathBuf, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const SAMPLE: &str = r#"garden "ai"

node "qmplyuym" {
    slug "viridian"
    created "2026-11-20T14:46:41Z"
    tags "project"
}

node "wopxuwkn" {
    slug "first-slice"
    created "2026-11-20T05:12:49Z"
    parent "qmplyuym"
    restaurant {
        url "https://example.com"
    }
}
"#;

    #[test]
    fn round_trips_the_canonical_format() {
        let meta = Meta::parse("viri.kdl".into(), SAMPLE).unwrap();
        assert_eq!(meta.garden_name(), "ai");
        assert_eq!(meta.to_text(), SAMPLE);
    }

    #[test]
    fn preserves_unknown_components() {
        let mut meta = Meta::parse("viri.kdl".into(), SAMPLE).unwrap();
        let block = meta
            .blocks
            .iter_mut()
            .find(|b| b.meta.slug.as_deref() == Some("first-slice"))
            .unwrap();
        block.meta.slug = Some("renamed".to_owned());
        let text = meta.to_text();
        assert!(text.contains("slug \"renamed\""));
        assert!(text.contains("    restaurant {\n        url \"https://example.com\"\n    }"));
    }

    #[test]
    fn ids_and_get() {
        let meta = Meta::parse("viri.kdl".into(), SAMPLE).unwrap();
        let id: NodeId = "qmplyuym".parse().unwrap();
        assert_eq!(
            meta.ids(),
            BTreeSet::from([id, "wopxuwkn".parse().unwrap()])
        );
        assert_eq!(meta.get(&id).unwrap().slug.as_deref(), Some("viridian"));
        assert_eq!(meta.get(&"zzzzzzzk".parse().unwrap()), None);
    }

    #[test]
    fn rejects_malformed_files() {
        for bad in [
            "node \"qmplyuym\" {\n}",
            "garden \"ai\"\nnode \"nope!\" {\n}\n",
            "garden \"ai\"\nnode \"qmplyuym\" {\n    slug \"a\"\n    created \"2026-11-20T14:46:41Z\"\n}\nnode \"qmplyuym\" {\n    slug \"b\"\n}\n",
            "garden \"ai\"\ngarden \"ai\"\n",
            "garden \"ai\"\nnode \"qmplyuym\" {\n    slug \"a\"\n}\n",
        ] {
            assert!(Meta::parse("viri.kdl".into(), bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn names_each_malformed_piece() {
        let cases = [
            ("garden \"ai\"\nnode \"qmplyuym\"\n", "has no `{ }` block"),
            (
                "garden \"ai\"\nnode \"qmplyuym\" {\n    slug \"a\"\n    created \"2026-11-20T14:46:41Z\"\n    created \"2026-11-21T10:00:00Z\"\n}\n",
                "two `created` lines",
            ),
            (
                "garden \"ai\"\nnode \"qmplyuym\" {\n    slug \"a\"\n    created \"2026-11-20T14:46:41Z\"\n    parent \"nope\"\n}\n",
                "is not a node id",
            ),
            (
                "garden \"ai\"\nnode \"qmplyuym\" {\n    slug \"a\"\n    created \"2026-11-20T14:46:41Z\"\n    tags \"a\" \"b\"\n}\n",
                "exactly one argument",
            ),
            (
                "garden \"ai\"\nnode \"qmplyuym\" {\n    slug 42\n    created \"2026-11-20T14:46:41Z\"\n}\n",
                "quoted string",
            ),
            (
                "garden \"ai\"\nnode \"qmplyuym\" {\n    slug \"a\"\n    created \"2026-11-20T14:46:41Z\"\n}\nnode \"qmplyuym\" {\n    slug \"b\"\n    created \"2026-11-20T14:46:41Z\"\n}\n",
                "appears twice",
            ),
        ];
        for (text, expected) in cases {
            let err = Meta::parse("viri.kdl".into(), text).expect_err("rejected");
            let MetaError::Invalid { message, .. } = err else {
                unreachable!();
            };
            assert!(
                message.contains(expected),
                "{message} should mention {expected}"
            );
        }
    }

    #[test]
    fn open_reports_missing_and_unparsable_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(dir.path().join("viri.kdl")).unwrap();
        assert!(matches!(Meta::open(&path), Err(MetaError::Io { .. })));
        fs_err::write(&path, "garden \"unterminated").unwrap();
        assert!(matches!(Meta::open(&path), Err(MetaError::Parse { .. })));
    }

    #[test]
    fn write_fails_when_the_filesystem_rejects_it() {
        let dir = tempfile::TempDir::new().unwrap();
        // A directory squatting on the tempfile path makes the write fail.
        let path = Utf8PathBuf::try_from(dir.path().join("viri.kdl")).unwrap();
        fs_err::create_dir(dir.path().join("viri.kdl.tmp")).unwrap();
        let meta = Meta::parse(path.clone(), SAMPLE).unwrap();
        assert!(matches!(meta.write(), Err(MetaError::Io { .. })));

        // A directory on the final path makes the rename fail.
        let dir = tempfile::TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(dir.path().join("viri.kdl")).unwrap();
        fs_err::create_dir(&path).unwrap();
        let meta = Meta::parse(path.clone(), SAMPLE).unwrap();
        assert!(matches!(meta.write(), Err(MetaError::Io { .. })));
    }

    #[test]
    fn renders_property_entries_and_nested_children_of_extras() {
        let text = concat!(
            "garden \"ai\"\n\n",
            "node \"qmplyuym\" {\n",
            "    slug \"x\"\n",
            "    created \"2026-11-20T14:46:41Z\"\n",
            "    restaurant \"mama\" url=\"https://example.com\"\n",
            "}\n",
        );
        let meta = Meta::parse("viri.kdl".into(), text).unwrap();
        let out = meta.to_text();
        assert!(
            out.contains("restaurant \"mama\" url=\"https://example.com\""),
            "{out}"
        );
    }
}

#[cfg(test)]
mod coverage {
    use super::tests::SAMPLE;
    use super::*;

    #[test]
    fn upserted_nodes_without_slug_or_created_render_without_them() {
        let mut meta = Meta::parse("viri.kdl".into(), SAMPLE).unwrap();
        let id: NodeId = "zzzzzzzk".parse().unwrap();
        meta.upsert(NodeMeta {
            id,
            slug: None,
            created: None,
            tags: vec![],
            parents: vec![],
        });
        let out = meta.to_text();
        let block = out.split("node \"zzzzzzzk\"").nth(1).unwrap();
        assert!(
            !block.contains("slug") && !block.contains("created"),
            "{out}"
        );
    }

    #[test]
    fn non_string_extra_values_render_as_literals() {
        let text = concat!(
            "garden \"ai\"\n\n",
            "node \"qmplyuym\" {\n",
            "    slug \"x\"\n",
            "    created \"2026-11-20T14:46:41Z\"\n",
            "    seating 4\n",
            "}\n",
        );
        let meta = Meta::parse("viri.kdl".into(), text).unwrap();
        assert!(meta.to_text().contains("seating 4"), "{}", meta.to_text());
    }
}
