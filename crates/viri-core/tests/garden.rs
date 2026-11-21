//! Integration tests for the garden join over a real jj repo.

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

use std::sync::Arc;

use camino::Utf8PathBuf;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::Workspace;
use tempfile::TempDir;
use viri_core::Garden;
use viri_core::NodeId;
use viri_core::body::Bodies;

fn settings() -> Arc<UserSettings> {
    let mut config = StackedConfig::with_defaults();
    let mut user = ConfigLayer::empty(ConfigSource::User);
    user.set_value("user.name", "Test").unwrap();
    user.set_value("user.email", "test@example.com").unwrap();
    user.set_value("operation.hostname", "test").unwrap();
    user.set_value("operation.username", "test").unwrap();
    config.add_layer(user);
    Arc::new(UserSettings::from_config_and_home_dir(config, None).unwrap())
}

async fn garden() -> (TempDir, Garden) {
    let dir = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
    Workspace::init_simple(&settings(), dir.path(), &WorkspaceNameBuf::from("default"))
        .await
        .unwrap();
    fs_err::write(
        root.join("viri.kdl"),
        concat!(
            "garden \"test\"\n",
            "\n",
            "node \"zkqpmzxy\" {\n",
            "    slug \"harissa\"\n",
            "    created \"2026-11-21T10:00:00Z\"\n",
            "}\n",
            "\n",
            "node \"tsrqponm\" {\n",
            "    slug \"pending-body\"\n",
            "    created \"2026-11-21T10:01:00Z\"\n",
            "    parent \"zkqpmzxy\"\n",
            "}\n",
        ),
    )
    .unwrap();

    let id: NodeId = "zkqpmzxy".parse().unwrap();
    let mut bodies = Bodies::open_with(settings(), &root).await.unwrap();
    bodies
        .write_body(&id, None, "harissa", "committed body\n", "new")
        .await
        .unwrap();
    drop(bodies);

    let garden = Garden::open(&root).await.unwrap();
    (dir, garden)
}

fn nid(s: &str) -> NodeId {
    s.parse().unwrap()
}

#[tokio::test]
async fn loading_an_unknown_node_fails() {
    let (_dir, garden) = garden().await;
    assert!(matches!(
        garden.load(&nid("zzzzzzzz")).await,
        Err(viri_core::garden::GardenError::UnknownNode(_))
    ));
}

#[tokio::test]
async fn saving_invalid_metadata_reports_every_error() {
    let (_dir, mut garden) = garden().await;
    let mut node = garden.load(&nid("zkqpmzxy")).await.unwrap();
    node.meta.slug = Some("Not Kebab".to_owned());
    node.meta.parents.push(nid("zzzzzzzz"));
    match garden.save(&node, "edit", Some("harissa")).await {
        Err(viri_core::garden::GardenError::Invalid(errors)) => {
            let paths: Vec<&str> = errors.iter().map(|e| e.path.as_str()).collect();
            assert_eq!(paths, vec!["slug", "parents[0]"]);
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn a_body_pending_node_renames_through_viri() {
    let (dir, mut garden) = garden().await;
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();

    // The node's slug changes while its body file doesn't exist yet; the
    // rename finds nothing to move and the content commit creates the file.
    let mut node = garden.load(&nid("tsrqponm")).await.unwrap();
    assert_eq!(node.body, "");
    node.meta.slug = Some("body-arrived".to_owned());
    node.body = "now it exists\n".to_owned();
    garden
        .save(&node, "edit", Some("pending-body"))
        .await
        .unwrap();

    assert!(root.join("tsrqponm body-arrived.viri").is_file());
    let reloaded = garden.load(&nid("tsrqponm")).await.unwrap();
    assert_eq!(reloaded.body, "now it exists\n");
    assert_eq!(
        reloaded.meta.slug.as_deref(),
        Some("body-arrived"),
        "the metadata keeps up with the rename"
    );
}
