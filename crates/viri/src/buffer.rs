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

/// Renders a buffer, optionally with validation errors injected as comments
/// at the top of the frontmatter.
pub fn render(buffer: &Buffer, errors: &[ValidationError]) -> String {
    let mut out = String::from("---\n");
    if !errors.is_empty() {
        out.push_str(&format!(
            "# viri: {} error(s), fix them and save again\n",
            errors.len()
        ));
        for error in errors {
            out.push_str(&format!("# {error}\n"));
        }
    }
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
