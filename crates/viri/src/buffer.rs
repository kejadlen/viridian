//! The edit buffer: a node as YAML frontmatter plus body text.

use std::str::FromStr;

use viri_core::id::NodeId;
use viri_core::schema::NodeMeta;
use viri_core::schema::ValidationError;

/// A parsed or rendered edit buffer.
#[derive(Clone, Debug)]
pub struct Buffer {
    pub meta: NodeMeta,
    pub body: String,
}

impl Buffer {
    pub fn new(meta: NodeMeta, body: String) -> Self {
        Self { meta, body }
    }
}

/// Renders a buffer as frontmatter plus body.
pub fn render(buffer: &Buffer) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("id: {}\n", buffer.meta.id));
    match buffer.meta.slug.as_deref() {
        Some(slug) => out.push_str(&format!("slug: {slug}\n")),
        None => out.push_str("slug:\n"),
    }
    if let Some(created) = buffer.meta.created {
        out.push_str(&format!("created: {}\n", format_timestamp(created)));
    } else {
        out.push_str("created:\n");
    }
    write_list(&mut out, "tags", &buffer.meta.tags);
    write_list(
        &mut out,
        "parents",
        &buffer
            .meta
            .parents
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>(),
    );
    out.push_str("---\n");
    out.push_str(&buffer.body);
    out
}

fn write_list(out: &mut String, key: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("{key}:\n"));
    for item in items {
        out.push_str(&format!("  - {item}\n"));
    }
}

fn format_timestamp(ts: jiff::Timestamp) -> String {
    jiff::Timestamp::from_second(ts.as_second())
        .unwrap_or(ts)
        .to_string()
}

/// Which list a `  - item` line belongs to.
#[derive(Clone, Copy, PartialEq)]
enum List {
    None,
    Tags,
    Parents,
}

/// Parses an edited buffer back into a node. `expected_id` pins the buffer to
/// the node being edited; every problem is reported as a validation error
/// with a field path.
pub fn parse(text: &str, expected_id: &NodeId) -> Result<Buffer, Vec<ValidationError>> {
    let mut errors = Vec::new();

    let (frontmatter, body) = match text.strip_prefix("---\n") {
        Some(rest) => match rest.split_once("\n---\n") {
            Some((frontmatter, body)) => (frontmatter, body.to_owned()),
            None => {
                return Err(vec![ValidationError::new(
                    "(buffer)",
                    "the frontmatter has no closing `---` line",
                )]);
            }
        },
        None => {
            return Err(vec![ValidationError::new(
                "(buffer)",
                "the buffer must start with a `---` line",
            )]);
        }
    };

    let mut slug: Option<String> = None;
    let mut created: Option<jiff::Timestamp> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut parents: Vec<String> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut list = List::None;

    for (line, number) in frontmatter.lines().zip(2..) {
        if line.starts_with('#') {
            continue;
        }
        if line.trim().is_empty() {
            list = List::None;
            continue;
        }
        if let Some(item) = line.strip_prefix("  - ") {
            let item = unquote(item, &mut errors);
            match list {
                List::Tags => tags.push(item),
                List::Parents => parents.push(item),
                List::None => errors.push(ValidationError::new(
                    format!("line {number}"),
                    "a list item with no list above it",
                )),
            }
            continue;
        }
        list = List::None;
        let Some((key, value)) = line.split_once(':') else {
            errors.push(ValidationError::new(
                format!("line {number}"),
                format!("expected `key: value`, got `{line}`"),
            ));
            continue;
        };
        let value = value.trim();
        if seen.contains(&key) {
            errors.push(ValidationError::new(key, format!("`{key}` appears twice")));
            continue;
        }
        seen.push(key);
        match key {
            "id" => match value.parse::<NodeId>() {
                Ok(id) if id == *expected_id => {}
                Ok(id) => errors.push(ValidationError::new(
                    "id",
                    format!("cannot change a node's id from {expected_id} to {id}"),
                )),
                Err(_) => errors.push(ValidationError::new(
                    "id",
                    format!("`{value}` is not a node id"),
                )),
            },
            "slug" => {
                let parsed = unquote(value, &mut errors);
                slug = Some(parsed);
            }
            "created" => {
                if value.is_empty() {
                    created = None;
                } else {
                    let parsed = unquote(value, &mut errors);
                    match jiff::Timestamp::from_str(&parsed) {
                        Ok(ts) => created = Some(ts),
                        Err(_) => errors.push(ValidationError::new(
                            "created",
                            format!("`{value}` is not an RFC 3339 timestamp"),
                        )),
                    }
                }
            }
            "tags" => {
                tags = parse_list(value, "tags", &mut errors);
                if value.is_empty() {
                    list = List::Tags;
                }
            }
            "parents" => {
                parents = parse_list(value, "parents", &mut errors);
                if value.is_empty() {
                    list = List::Parents;
                }
            }
            _ => errors.push(ValidationError::new(
                key.to_owned(),
                format!("unknown field `{key}`"),
            )),
        }
    }

    let mut parsed_parents = Vec::new();
    for (i, raw) in parents.iter().enumerate() {
        match NodeId::from_str(raw) {
            Ok(id) => parsed_parents.push(id),
            Err(_) => errors.push(ValidationError::new(
                format!("parents[{i}]"),
                format!("`{raw}` is not a node id"),
            )),
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(Buffer {
        meta: NodeMeta {
            id: *expected_id,
            slug: slug.filter(|s| !s.is_empty()),
            created,
            tags,
            parents: parsed_parents,
        },
        body,
    })
}

fn parse_list(value: &str, key: &str, errors: &mut Vec<ValidationError>) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        errors.push(ValidationError::new(
            key,
            format!("`{value}` is neither a list like `[a, b]` nor empty"),
        ));
        return Vec::new();
    };
    inner
        .split(',')
        .map(|item| unquote(item.trim(), errors))
        .collect()
}

fn unquote(value: &str, errors: &mut Vec<ValidationError>) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner.to_owned();
        }
    }
    if value.contains(char::is_whitespace) {
        errors.push(ValidationError::new(
            "(buffer)",
            format!("`{value}` contains whitespace; quote the whole value if it's one field"),
        ));
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn id() -> NodeId {
        "zkqpmzxy".parse().unwrap()
    }

    pub(super) fn errors_of(text: &str) -> Vec<ValidationError> {
        parse(text, &id()).unwrap_err()
    }

    #[test]
    fn renders_a_node_without_a_slug_or_created() {
        let meta = NodeMeta {
            id: id(),
            slug: None,
            created: None,
            tags: vec![],
            parents: vec![],
        };
        let text = render(&Buffer::new(meta, String::new()));
        assert!(text.contains("slug:\n"), "{text}");
        assert!(text.contains("created:\n"), "{text}");
    }

    #[test]
    fn rejects_an_orphan_list_item() {
        let errors = errors_of("---\nslug: x\n  - stray\n---\n");
        assert!(
            errors
                .iter()
                .any(|e| e.path == "line 3" && e.message.contains("no list above it")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_a_line_without_a_colon() {
        let errors = errors_of("---\nnonsense\n---\n");
        assert!(
            errors
                .iter()
                .any(|e| e.path == "line 2" && e.message.contains("key: value")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_an_unparsable_id() {
        let errors = errors_of("---\nid: nope!\nslug: x\n---\n");
        assert!(
            errors
                .iter()
                .any(|e| e.path == "id" && e.message.contains("not a node id")),
            "{errors:?}"
        );
    }

    #[test]
    fn accepts_an_empty_created() {
        let parsed = parse("---\nslug: x\ncreated:\n---\nbody\n", &id()).unwrap();
        assert_eq!(parsed.meta.created, None);
        assert_eq!(parsed.body, "body\n");
    }

    #[test]
    fn rejects_an_unparsable_parent() {
        let errors = errors_of("---\nslug: x\nparents:\n  - nope\n---\n");
        assert!(
            errors
                .iter()
                .any(|e| e.path == "parents[0]" && e.message.contains("not a node id")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_an_unquotable_whitespace_value() {
        let errors = errors_of("---\nslug: two words\n---\n");
        assert!(
            errors
                .iter()
                .any(|e| e.path == "(buffer)" && e.message.contains("whitespace")),
            "{errors:?}"
        );
    }

    #[test]
    fn accepts_quoted_values() {
        let parsed = parse("---\nslug: \"two words\"\n---\n", &id()).unwrap();
        assert_eq!(parsed.meta.slug.as_deref(), Some("two words"));
    }
}

#[cfg(test)]
mod coverage {
    use super::tests::*;

    #[test]
    fn rejects_a_missing_closing_fence() {
        let errors = errors_of("---\nslug: x\n");
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("no closing `---`")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_blank_line_ends_a_list() {
        let errors = errors_of("---\ntags:\n\n  - orphan\n---\n");
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("no list above it")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_an_unparsable_created() {
        let errors = errors_of("---\nslug: x\ncreated: not-a-time\n---\n");
        assert!(
            errors
                .iter()
                .any(|e| e.path == "created" && e.message.contains("RFC 3339")),
            "{errors:?}"
        );
    }
}
