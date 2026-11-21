//! Property tests for node ids and the edit buffer.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

use std::collections::BTreeSet;

use hegel::Generator;
use hegel::TestCase;
use hegel::generators as gs;
use hegel::test;
use viri::buffer;
use viri_core::id::NodeId;
use viri_core::id::resolve_prefix;
use viri_core::id::shortest_prefix;
use viri_core::schema::NodeMeta;

/// Draws a node id from a random u32.
fn node_id(tc: &TestCase) -> NodeId {
    let n = tc.draw(gs::integers::<u32>());
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&n.to_be_bytes());
    NodeId::from_bytes(bytes)
}

#[test]
fn ids_round_trip_through_their_display_form(tc: TestCase) {
    let id = node_id(&tc);
    let text = id.to_string();
    assert_eq!(text.len(), 8);
    assert!(text.bytes().all(|b| (b'k'..=b'z').contains(&b)), "{text}");
    assert_eq!(text.parse::<NodeId>().unwrap(), id);
}

#[test]
fn shortest_prefixes_resolve_back_to_their_id(tc: TestCase) {
    let ids: BTreeSet<NodeId> = tc
        .draw(gs::vecs(id_gen()).max_size(50).unique(true))
        .into_iter()
        .collect();
    if ids.is_empty() {
        return;
    }
    for id in &ids {
        let prefix = shortest_prefix(id, &ids);
        assert!(prefix.len() <= 8);
        assert_eq!(resolve_prefix(&prefix, &ids).unwrap(), *id);
    }
}

#[test]
fn buffers_round_trip_through_render_and_parse(tc: TestCase) {
    let id = node_id(&tc);
    let tags: Vec<String> = tc
        .draw(
            gs::vecs(gs::from_regex("[a-z0-9-]{1,8}"))
                .max_size(4)
                .unique(true),
        )
        .into_iter()
        .collect();
    let parents: Vec<NodeId> = tc
        .draw(gs::vecs(id_gen()).max_size(3).unique(true))
        .into_iter()
        .collect();
    let body = tc.draw(gs::from_regex("[ -~\n\t]{0,60}"));

    let meta = NodeMeta {
        id,
        slug: Some("round-trip".to_owned()),
        created: Some(viri_core::schema::now()),
        tags,
        parents,
    };
    let rendered = buffer::render(&buffer::Buffer::new(meta.clone(), body.clone()));
    let parsed = buffer::parse(&rendered, &id).unwrap();
    assert_eq!(parsed.meta, meta);
    assert_eq!(parsed.body, body);
}

#[test]
fn a_changed_id_is_always_rejected(tc: TestCase) {
    let id = node_id(&tc);
    let other = node_id(&tc);
    if id == other {
        return;
    }
    let rendered = buffer::render(&buffer::Buffer::new(
        NodeMeta {
            id,
            slug: Some("x".to_owned()),
            created: Some(viri_core::schema::now()),
            tags: vec![],
            parents: vec![],
        },
        String::new(),
    ));
    let tampered = rendered.replace(&id.to_string(), &other.to_string());
    let errors = buffer::parse(&tampered, &id).unwrap_err();
    assert!(errors.iter().any(|e| e.path == "id"), "{errors:?}");
}

#[test]
fn bodies_containing_frontmatter_fences_round_trip_verbatim(tc: TestCase) {
    let id = node_id(&tc);
    let pre = tc.draw(gs::from_regex("[a-zA-Z ,.!]{0,20}"));
    let post = tc.draw(gs::from_regex("[a-zA-Z ,.!\n]{0,20}"));
    let body = format!("{pre}\n---\n{post}");

    let meta = NodeMeta {
        id,
        slug: Some("fenced".to_owned()),
        created: Some(viri_core::schema::now()),
        tags: vec![],
        parents: vec![],
    };
    let rendered = buffer::render(&buffer::Buffer::new(meta, body.clone()));
    let parsed = buffer::parse(&rendered, &id).unwrap();
    assert_eq!(parsed.body, body, "the first fence closes the frontmatter");
}

#[test]
fn fractional_timestamps_render_truncated_and_stay_truncated(tc: TestCase) {
    let id = node_id(&tc);
    let second = tc.draw(gs::integers::<i64>().min_value(0).max_value(4_102_444_800));
    let nanosecond = tc.draw(gs::integers::<i32>().min_value(1).max_value(999_999_999));
    let created = jiff::Timestamp::new(second, nanosecond).unwrap();
    let truncated = jiff::Timestamp::from_second(second).unwrap();

    let meta = NodeMeta {
        id,
        slug: Some("truncated".to_owned()),
        created: Some(created),
        tags: vec![],
        parents: vec![],
    };
    let rendered = buffer::render(&buffer::Buffer::new(meta, String::new()));
    let parsed = buffer::parse(&rendered, &id).unwrap();
    assert_eq!(parsed.meta.created, Some(truncated));
}

#[test]
fn flow_lists_and_block_lists_parse_identically(tc: TestCase) {
    let id = node_id(&tc);
    let tags: Vec<String> = tc
        .draw(
            gs::vecs(gs::from_regex("[a-z0-9-]{1,8}"))
                .max_size(4)
                .unique(true),
        )
        .into_iter()
        .collect();
    let parents: Vec<String> = tc
        .draw(
            gs::vecs(gs::from_regex("[k-z]{8}"))
                .min_size(1)
                .max_size(3)
                .unique(true),
        )
        .into_iter()
        .collect();

    let mut block = String::from("---\n");
    block.push_str(&format!(
        "id: {id}\nslug: flow-vs-block\ncreated: 2026-11-20T14:46:41Z\n"
    ));
    if !tags.is_empty() {
        block.push_str(&format!("tags: [{}]\n", tags.join(", ")));
    }
    block.push_str(&format!("parents: [{}]\n", parents.join(", ")));
    block.push_str("---\n");

    let mut spaced = block.clone();
    if !tags.is_empty() {
        let flow = format!("tags: [{}]\n", tags.join(", "));
        let block_list = format!(
            "tags:\n{}",
            tags.iter()
                .map(|t| format!("  - {t}\n"))
                .collect::<String>()
        );
        spaced = spaced.replace(&flow, &block_list);
    }
    {
        let flow = format!("parents: [{}]\n", parents.join(", "));
        let block_list = format!(
            "parents:\n{}",
            parents
                .iter()
                .map(|p| format!("  - {p}\n"))
                .collect::<String>()
        );
        spaced = spaced.replace(&flow, &block_list);
    }

    let from_flow = buffer::parse(&block, &id).unwrap();
    let from_block = buffer::parse(&spaced, &id).unwrap();
    assert_eq!(from_flow.meta.tags, from_block.meta.tags);
    assert_eq!(from_flow.meta.parents, from_block.meta.parents);
}

#[test]
fn tampered_frontmatter_is_rejected_at_the_offending_key(tc: TestCase) {
    let id = node_id(&tc);
    let mut text = buffer::render(&buffer::Buffer::new(
        NodeMeta {
            id,
            slug: Some("x".to_owned()),
            created: Some(viri_core::schema::now()),
            tags: vec![],
            parents: vec![],
        },
        String::new(),
    ));
    let corruption = tc.draw(gs::integers::<usize>().max_value(4));
    match corruption {
        0 => text = text.replacen("slug:", "wrong: none", 1),
        1 => text = text.replacen("slug: x", "slug: x\nslug: y", 1),
        2 => {
            text = text.replacen("---\n", "", 1);
        }
        _ => {
            text = text.replacen("slug:", "tags: not-a-list\nslug:", 1);
        }
    }
    let errors = buffer::parse(&text, &id).unwrap_err();
    let expected_path = match corruption {
        0 => "wrong",
        1 => "slug",
        2 => "(buffer)",
        _ => "tags",
    };
    assert!(
        errors.iter().any(|e| e.path == expected_path),
        "corruption {corruption} should flag `{expected_path}`, got {errors:?}"
    );
}

fn id_gen() -> impl hegel::PrintableGenerator<NodeId> {
    gs::integers::<u32>().map(|n| {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&n.to_be_bytes());
        NodeId::from_bytes(bytes)
    })
}
