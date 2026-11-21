//! Property tests for viri-core, derived from the modules' contracts:
//! doc comments, the garden's design notes, and the type signatures.

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

use camino::Utf8PathBuf;
use hegel::Generator;
use hegel::TestCase;
use hegel::generators as gs;
use hegel::test;
use jiff::Timestamp;
use tempfile::TempDir;
use viri_core::id::NodeId;
use viri_core::id::PrefixError;
use viri_core::id::resolve_prefix;
use viri_core::id::shortest_prefix;
use viri_core::meta::Meta;
use viri_core::schema::NodeMeta;

const KEBAK: &str = "[a-z0-9]+(-[a-z0-9]+)*";

fn nid(tc: &TestCase) -> NodeId {
    let n = tc.draw(gs::integers::<u32>());
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&n.to_be_bytes());
    NodeId::from_bytes(bytes)
}

fn nid_gen() -> impl hegel::PrintableGenerator<NodeId> {
    gs::integers::<u32>().map(|n| {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&n.to_be_bytes());
        NodeId::from_bytes(bytes)
    })
}

fn id_set_gen() -> impl hegel::PrintableGenerator<BTreeSet<NodeId>> {
    gs::vecs(nid_gen())
        .max_size(40)
        .unique(true)
        .map(|v| v.into_iter().collect())
}

fn slug(tc: &TestCase) -> String {
    tc.draw(gs::from_regex(KEBAK))
}

fn timestamp(tc: &TestCase) -> Timestamp {
    let second = tc.draw(
        gs::integers::<i64>()
            .min_value(-62_167_219_200) // 0001-01-01, jiff's civil lower bound
            .max_value(4_102_444_800), // 2100-01-01
    );
    let nanosecond = tc.draw(gs::integers::<i32>().min_value(0).max_value(999_999_999));
    Timestamp::new(second, nanosecond).unwrap()
}

// --- id: reject direction -------------------------------------------------

#[test]
fn malformed_ids_are_always_rejected(tc: TestCase) {
    // Corrupt one or two positions of an otherwise valid id spelling.
    let base = nid(&tc).to_string().into_bytes();
    let mut corrupted = base;
    let position = tc.draw(gs::integers::<usize>().max_value(7));
    let bad = tc.draw(gs::sampled_from(vec![
        b'a',
        b'0',
        b'j',
        b'z' + 1,
        b' ',
        b'-',
        0xC3,
    ]));
    corrupted[position] = bad;
    if tc.draw(gs::booleans()) {
        let other = tc.draw(gs::integers::<usize>().max_value(7));
        corrupted[other] = b'j';
    }
    let Ok(s) = String::from_utf8(corrupted) else {
        // Invalid UTF-8 cannot be a node id at all.
        return;
    };
    assert!(s.parse::<NodeId>().is_err(), "{s} should be invalid");
}

#[test]
fn ids_of_the_wrong_length_are_rejected(tc: TestCase) {
    let text = tc.draw(gs::from_regex("[k-z]{1,12}"));
    let valid = text.len() == 8 && text.bytes().all(|b| (b'k'..=b'z').contains(&b));
    assert_eq!(text.parse::<NodeId>().is_ok(), valid, "{text}");
}

// --- id: prefix resolution --------------------------------------------------

#[test]
fn prefix_resolution_agrees_with_the_matches_it_names(tc: TestCase) {
    let ids = tc.draw(id_set_gen());
    let prefix = tc.draw(gs::from_regex("[k-z]{0,9}"));
    let matches: Vec<NodeId> = ids
        .iter()
        .filter(|id| id.to_string().starts_with(&prefix))
        .copied()
        .collect();
    match resolve_prefix(&prefix, &ids) {
        Ok(id) => assert_eq!(matches.as_slice(), &[id]),
        Err(PrefixError::NotFound(p)) => {
            assert_eq!(p, prefix);
            assert!(matches.is_empty());
        }
        Err(PrefixError::Ambiguous {
            prefix: p,
            matches: m,
        }) => {
            assert_eq!(p, prefix);
            assert_eq!(m, matches);
        }
    }
}

#[test]
fn the_empty_prefix_matches_every_id(tc: TestCase) {
    let ids = tc.draw(id_set_gen());
    match ids.len() {
        0 => assert_eq!(
            resolve_prefix("", &ids).unwrap_err(),
            PrefixError::NotFound(String::new())
        ),
        1 => {
            let only = *ids.iter().next().unwrap();
            assert_eq!(resolve_prefix("", &ids).unwrap(), only);
        }
        _ => {
            let matches: Vec<NodeId> = ids.iter().copied().collect();
            assert_eq!(
                resolve_prefix("", &ids).unwrap_err(),
                PrefixError::Ambiguous {
                    prefix: String::new(),
                    matches,
                }
            );
        }
    }
}

#[test]
fn shortest_prefixes_are_minimal(tc: TestCase) {
    let ids = tc.draw(id_set_gen());
    if ids.is_empty() {
        return;
    }
    let id = tc.draw(gs::sampled_from(ids.iter().copied().collect::<Vec<_>>()));
    let prefix = shortest_prefix(&id, &ids);
    assert!(prefix.len() <= 8);
    if let Some(shorter) = prefix
        .as_str()
        .get(..prefix.len().saturating_sub(1))
        .filter(|s| !s.is_empty())
    {
        // A proper prefix of the shortest unique prefix still matches at
        // least two ids, or it would have been returned instead.
        let sharing = ids
            .iter()
            .filter(|other| other.to_string().starts_with(shorter))
            .count();
        assert!(sharing >= 2, "`{shorter}` was already unique");
    }
}

// --- schema -----------------------------------------------------------------

#[test]
fn valid_metadata_validates_clean(tc: TestCase) {
    let ids = tc.draw(id_set_gen());
    let id = nid(&tc);
    let parents: Vec<NodeId> = ids.iter().copied().collect();
    let tags: Vec<String> = tc
        .draw(
            gs::vecs(gs::from_regex("[a-z0-9-]{1,10}"))
                .max_size(5)
                .unique(true),
        )
        .into_iter()
        .collect();
    let meta = NodeMeta {
        id,
        slug: Some(slug(&tc)),
        created: Some(timestamp(&tc)),
        tags,
        parents,
    };
    assert!(meta.validate(&ids).is_empty());
}

#[test]
fn bad_slugs_are_flagged_at_the_slug_path(tc: TestCase) {
    let ids = tc.draw(id_set_gen());
    let bad = tc.draw(gs::sampled_from(vec![
        "",
        "Harissa",
        "hot sauce",
        "-harissa",
        "harissa-",
        "a--b",
        "a_b",
        "a.b",
    ]));
    let meta = NodeMeta {
        id: nid(&tc),
        slug: Some(bad.to_owned()),
        created: Some(timestamp(&tc)),
        tags: vec![],
        parents: vec![],
    };
    let errors = meta.validate(&ids);
    assert!(
        errors.iter().any(|e| e.path == "slug"),
        "{bad} should fail at slug, got {errors:?}"
    );
}

#[test]
fn bad_components_are_flagged_at_their_index(tc: TestCase) {
    let known_id = nid(&tc);
    let known: BTreeSet<NodeId> = [known_id].into_iter().collect();
    let unknown: NodeId = flip_last_char(&known_id.to_string()).parse().unwrap();
    let meta = NodeMeta {
        id: known_id,
        slug: Some("x".to_owned()),
        created: Some(Timestamp::UNIX_EPOCH),
        tags: vec!["ok".to_owned(), "two words".to_owned()],
        parents: vec![known_id, unknown, unknown],
    };
    let errors = meta.validate(&known);
    assert!(errors.iter().any(|e| e.path == "tags[1]"), "{errors:?}");
    assert!(
        !errors.iter().any(|e| e.path == "parents[0]"),
        "a known, unique parent is fine: {errors:?}"
    );
    for i in [1, 2] {
        let at: Vec<&viri_core::schema::ValidationError> = errors
            .iter()
            .filter(|e| e.path == format!("parents[{i}]"))
            .collect();
        assert!(
            at.iter().any(|e| e.message.contains("unknown node id")),
            "{errors:?}"
        );
        assert!(
            at.iter().any(|e| e.message == "duplicate parent"),
            "{errors:?}"
        );
    }
}

// --- meta -------------------------------------------------------------------

struct GeneratedGarden {
    text: String,
    slugs: Vec<String>,
}

fn garden_text(tc: &TestCase) -> GeneratedGarden {
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(8));
    let mut slugs = Vec::new();
    let mut blocks = Vec::new();
    let mut pool: Vec<NodeId> = Vec::new();
    for i in 0..count {
        // Ids are distinct by construction, as in any real garden.
        let id = NodeId::from_bytes([0, 0, 0, u8::try_from(i).unwrap_or(u8::MAX)]);
        let slug = slug(tc);
        let created = timestamp(tc);
        let tags: Vec<String> = tc
            .draw(
                gs::vecs(gs::from_regex("[a-z0-9-]{1,8}"))
                    .max_size(3)
                    .unique(true),
            )
            .into_iter()
            .collect();
        let parents: Vec<String> = pool
            .iter()
            .filter(|_| tc.draw(gs::booleans()))
            .map(|id| id.to_string())
            .collect();
        let mut block =
            format!("node \"{id}\" {{\n    slug \"{slug}\"\n    created \"{created}\"\n");
        for tag in &tags {
            block.push_str(&format!("    tags \"{tag}\"\n"));
        }
        for parent in &parents {
            block.push_str(&format!("    parent \"{parent}\"\n"));
        }
        block.push_str("}\n");
        blocks.push(block);
        slugs.push(slug);
        pool.push(id);
    }
    GeneratedGarden {
        text: format!("garden \"test\"\n\n{}", blocks.join("\n")),
        slugs,
    }
}

fn open_in_tempdir(text: &str) -> (TempDir, Meta) {
    let dir = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(dir.path().join("viri.kdl")).unwrap();
    fs_err::write(&path, text).unwrap();
    let meta = Meta::open(&path).unwrap();
    (dir, meta)
}

fn delete_first_line_containing(text: &str, needle: &str) -> String {
    let start = text.find(needle).unwrap();
    let end = start + text[start..].find('\n').unwrap();
    format!("{}{}", &text[..start], &text[end..])
}

#[test]
fn generated_gardens_round_trip_through_the_file(tc: TestCase) {
    let generated = garden_text(&tc);
    let (_dir, meta) = open_in_tempdir(&generated.text);
    assert_eq!(
        meta.nodes()
            .map(|n| n.slug.clone().unwrap())
            .collect::<Vec<_>>(),
        generated.slugs,
        "file order must be creation order"
    );
    meta.write().unwrap();
    let dir = _dir; // keep the tempdir alive through the read below
    let rewritten =
        fs_err::read_to_string(Utf8PathBuf::try_from(dir.path().join("viri.kdl")).unwrap())
            .unwrap();
    assert_eq!(rewritten, generated.text);
}

#[test]
fn upsert_replaces_in_place_and_appends_at_the_end(tc: TestCase) {
    let generated = garden_text(&tc);
    let (_dir, mut meta) = open_in_tempdir(&generated.text);
    let before: Vec<NodeId> = meta.nodes().map(|n| n.id).collect();
    let Some(&target) = before.first() else {
        return;
    };
    let mut replaced = meta.get(&target).unwrap().clone();
    replaced.slug = Some("replaced-slug".to_owned());
    meta.upsert(replaced);
    let after: Vec<NodeId> = meta.nodes().map(|n| n.id).collect();
    assert_eq!(after, before, "replacing must not reorder");
    assert_eq!(
        meta.get(&target).unwrap().slug.as_deref(),
        Some("replaced-slug")
    );

    let fresh_id = flip_last_char(&target.to_string()).parse().unwrap();
    meta.upsert(NodeMeta::new(fresh_id));
    let after: Vec<NodeId> = meta.nodes().map(|n| n.id).collect();
    assert_eq!(after.last().copied(), Some(fresh_id), "new nodes append");
    assert_eq!(after.len() - 1, before.len());
}

/// A copy of `id`'s spelling whose last character differs, so it parses to a
/// different id without another draw.
fn flip_last_char(id: &str) -> String {
    let stem = &id[..id.len() - 1];
    match id.as_bytes()[id.len() - 1] {
        b'k' => format!("{stem}z"),
        _ => format!("{stem}k"),
    }
}

#[test]
fn near_valid_gardens_are_rejected(tc: TestCase) {
    let generated = garden_text(&tc);
    let corruption = tc.draw(gs::integers::<usize>().max_value(7));
    let text = match corruption {
        0 => delete_first_line_containing(&generated.text, "    slug \""),
        1 => generated.text.replacen("}\n", "    slug \"x\"\n}\n", 1),
        2 => generated
            .text
            .replacen("created \"", "created \"not-a-time", 1),
        3 => generated.text.replacen("node \"", "node \"nope!", 1),
        4 => generated
            .text
            .replacen("slug \"", "slug 42\n    xslug \"", 1),
        5 => format!(
            "{}\n{}",
            generated.text.trim_end(),
            generated.text.trim_end()
        ),
        6 => format!("{}\n{}", generated.text.trim_end(), "garden \"again\""),
        _ => format!("{}\n{}", generated.text.trim_end(), "stray \"value\""),
    };
    let dir = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(dir.path().join("viri.kdl")).unwrap();
    fs_err::write(&path, &text).unwrap();
    assert!(
        Meta::open(&path).is_err(),
        "corruption {corruption} should be rejected:\n{text}"
    );
}

// --- body -------------------------------------------------------------------

#[tokio::test]
#[hegel::test(test_cases = 12)]
async fn bodies_round_trip_through_a_real_jj_repo(tc: TestCase) {
    use jj_lib::config::ConfigLayer;
    use jj_lib::config::ConfigSource;
    use jj_lib::config::StackedConfig;
    use jj_lib::ref_name::WorkspaceNameBuf;
    use jj_lib::settings::UserSettings;
    use jj_lib::workspace::Workspace;
    use viri_core::body::Bodies;

    let mut config = StackedConfig::with_defaults();
    let mut user = ConfigLayer::empty(ConfigSource::User);
    user.set_value("user.name", "Test").unwrap();
    user.set_value("user.email", "test@example.com").unwrap();
    user.set_value("operation.hostname", "test").unwrap();
    user.set_value("operation.username", "test").unwrap();
    config.add_layer(user);
    let settings =
        std::sync::Arc::new(UserSettings::from_config_and_home_dir(config, None).unwrap());

    let dir = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
    Workspace::init_simple(&settings, dir.path(), &WorkspaceNameBuf::from("default"))
        .await
        .unwrap();
    let mut bodies = Bodies::open_with(settings, &root).await.unwrap();

    let id = nid(&tc);
    let slug = slug(&tc);
    let content = tc.draw(gs::from_regex("[a-zA-Z0-9 ,.!?\\n]{1,80}"));
    bodies
        .write_body(&id, None, &slug, &content, "new")
        .await
        .unwrap();
    assert_eq!(bodies.read_body(&id).unwrap(), content);
    assert!(
        root.join(viri_core::body::body_file_name(&id, &slug))
            .is_file()
    );
}
