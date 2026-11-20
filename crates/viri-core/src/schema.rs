use std::collections::BTreeSet;

use jiff::Timestamp;

use crate::id::NodeId;

/// A validation error, located by the frontmatter field or component it
/// concerns, such as `slug` or `parents[1]`.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error, miette::Diagnostic)]
#[error("{path}: {message}")]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// A node's metadata: its core fields plus the `tags` and `parent`
/// components. The body is not metadata; it lives in the garden's jj repo.
///
/// `slug` and `created` are required by validation but optional here, because
/// a node being edited can drop them from its frontmatter.
#[derive(Clone, PartialEq, Debug)]
pub struct NodeMeta {
    pub id: NodeId,
    pub slug: Option<String>,
    pub created: Option<Timestamp>,
    pub tags: Vec<String>,
    pub parents: Vec<NodeId>,
}

impl NodeMeta {
    /// A new node's starting metadata: no slug yet, created now.
    pub fn new(id: NodeId) -> Self {
        Self {
            id,
            slug: None,
            created: Some(now()),
            tags: Vec::new(),
            parents: Vec::new(),
        }
    }

    /// Validates this metadata against the ids known to the garden. Returns
    /// every violation, not just the first.
    pub fn validate(&self, known: &BTreeSet<NodeId>) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let slug = self.slug.as_deref().unwrap_or_default();
        if slug.is_empty() {
            errors.push(ValidationError::new("slug", "required"));
        } else if !is_kebab(slug) {
            errors.push(ValidationError::new(
                "slug",
                "expected kebab-case: lowercase letters, digits, and hyphens",
            ));
        }
        if self.created.is_none() {
            errors.push(ValidationError::new("created", "required"));
        }
        for (i, tag) in self.tags.iter().enumerate() {
            if tag.is_empty() || tag.chars().any(char::is_whitespace) {
                errors.push(ValidationError::new(
                    format!("tags[{i}]"),
                    "tags must be single words without whitespace",
                ));
            }
        }
        for (i, parent) in self.parents.iter().enumerate() {
            if !known.contains(parent) {
                errors.push(ValidationError::new(
                    format!("parents[{i}]"),
                    format!("unknown node id {parent}"),
                ));
            }
            if self.parents.iter().filter(|p| *p == parent).count() > 1 {
                errors.push(ValidationError::new(
                    format!("parents[{i}]"),
                    "duplicate parent",
                ));
            }
        }
        errors
    }
}

/// The current time, truncated to whole seconds to match the timestamps
/// already in the garden.
pub fn now() -> Timestamp {
    Timestamp::from_second(Timestamp::now().as_second()).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn is_kebab(s: &str) -> bool {
    !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, slug: Option<&str>, parents: Vec<&str>) -> NodeMeta {
        NodeMeta {
            id: id.parse().unwrap(),
            slug: slug.map(str::to_owned),
            created: Some(now()),
            tags: vec![],
            parents: parents.iter().map(|p| p.parse().unwrap()).collect(),
        }
    }

    #[test]
    fn valid_meta_has_no_errors() {
        let known = BTreeSet::from(["qmplyuym".parse().unwrap()]);
        assert!(
            meta("zkqpmzxy", Some("harissa"), vec!["qmplyuym"])
                .validate(&known)
                .is_empty()
        );
    }

    #[test]
    fn collects_every_violation() {
        let known = BTreeSet::from(["qmplyuym".parse().unwrap()]);
        let mut m = meta("zkqpmzxy", None, vec!["zzzzzzzz"]);
        m.created = None;
        m.tags = vec!["two words".to_owned()];
        let errors = m.validate(&known);
        let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            vec![
                "slug: required",
                "created: required",
                "tags[0]: tags must be single words without whitespace",
                "parents[0]: unknown node id zzzzzzzz",
            ]
        );
    }

    #[test]
    fn slugs_must_be_kebab_case() {
        let known = BTreeSet::new();
        for bad in ["Harissa", "hot sauce", "-harissa", "harissa-", "a--b"] {
            assert!(
                !meta("zkqpmzxy", Some(bad), vec![])
                    .validate(&known)
                    .is_empty(),
                "{bad} should fail validation"
            );
        }
    }
}
