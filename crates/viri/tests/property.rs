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
    let body = tc.draw(gs::from_regex("[a-zA-Z ,.!?\\n]{0,60}"));

    let meta = NodeMeta {
        id,
        slug: Some("round-trip".to_owned()),
        created: Some(viri_core::schema::now()),
        tags,
        parents,
    };
    let rendered = buffer::render(&buffer::Buffer::new(meta.clone(), body.clone()), &[]);
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
    let rendered = buffer::render(
        &buffer::Buffer::new(
            NodeMeta {
                id,
                slug: Some("x".to_owned()),
                created: Some(viri_core::schema::now()),
                tags: vec![],
                parents: vec![],
            },
            String::new(),
        ),
        &[],
    );
    let tampered = rendered.replace(&id.to_string(), &other.to_string());
    let errors = buffer::parse(&tampered, &id).unwrap_err();
    assert!(errors.iter().any(|e| e.path == "id"), "{errors:?}");
}

fn id_gen() -> impl hegel::PrintableGenerator<NodeId> {
    gs::integers::<u32>().map(|n| {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&n.to_be_bytes());
        NodeId::from_bytes(bytes)
    })
}
