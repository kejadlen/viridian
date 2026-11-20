//! Integration tests for the body store against real jj repos.

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
use jj_lib::repo::Repo;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::DefaultWorkspaceLoaderFactory;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceLoaderFactory as _;
use tempfile::TempDir;
use viri_core::body::Bodies;
use viri_core::id::NodeId;

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

async fn garden() -> (TempDir, Bodies) {
    let dir = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
    Workspace::init_simple(&settings(), dir.path(), &WorkspaceNameBuf::from("default"))
        .await
        .unwrap();
    let bodies = Bodies::open_with(settings(), &root).await.unwrap();
    (dir, bodies)
}

fn nid(s: &str) -> NodeId {
    s.parse().unwrap()
}

/// The commit descriptions from `@` down to the root.
async fn log(bodies: &Bodies) -> Vec<String> {
    let loader = DefaultWorkspaceLoaderFactory
        .create(bodies.root().as_std_path())
        .unwrap();
    let workspace = loader
        .load(
            &settings(),
            &jj_lib::default_backend_factories::default_backend_factories(),
            &jj_lib::default_backend_factories::default_working_copy_factories(),
        )
        .unwrap();
    let repo = workspace
        .repo_loader()
        .load_at_head(Some(workspace.workspace_name()))
        .await
        .unwrap();
    let mut descriptions = Vec::new();
    let mut commit = repo
        .store()
        .get_commit_async(
            repo.view()
                .get_wc_commit_id(workspace.workspace_name())
                .unwrap(),
        )
        .await
        .unwrap();
    loop {
        descriptions.push(commit.description().to_owned());
        let Some(parent_id) = commit.parent_ids().first() else {
            break;
        };
        if *parent_id == *repo.store().root_commit_id() {
            break;
        }
        commit = repo.store().get_commit_async(parent_id).await.unwrap();
    }
    descriptions
}

#[tokio::test]
async fn writes_a_body_below_the_working_copy() {
    let (dir, mut bodies) = garden().await;
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
    let id = nid("zkqpmzxy");

    bodies
        .write_body(&id, None, "harissa", "first body\n", "new")
        .await
        .unwrap();

    let file = root.join("zkqpmzxy harissa.viri");
    assert_eq!(fs_err::read_to_string(&file).unwrap(), "first body\n");
    assert_eq!(bodies.read_body(&id).unwrap(), "first body\n");
    assert_eq!(log(&bodies).await, vec!["", "viri: new harissa"]);
}

#[tokio::test]
async fn writes_a_changed_body_as_a_new_commit() {
    let (_dir, mut bodies) = garden().await;
    let id = nid("zkqpmzxy");
    bodies
        .write_body(&id, None, "harissa", "one\n", "new")
        .await
        .unwrap();
    bodies
        .write_body(&id, Some("harissa"), "harissa", "two\n", "edit")
        .await
        .unwrap();
    assert_eq!(
        log(&bodies).await,
        vec!["", "viri: edit harissa", "viri: new harissa"]
    );
}

#[tokio::test]
async fn an_unchanged_body_writes_nothing() {
    let (_dir, mut bodies) = garden().await;
    let id = nid("zkqpmzxy");
    bodies
        .write_body(&id, None, "harissa", "same\n", "new")
        .await
        .unwrap();
    bodies
        .write_body(&id, Some("harissa"), "harissa", "same\n", "edit")
        .await
        .unwrap();
    assert_eq!(log(&bodies).await, vec!["", "viri: new harissa"]);
}

#[tokio::test]
async fn renames_a_stale_slug_in_its_own_commit() {
    let (dir, mut bodies) = garden().await;
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
    let id = nid("zkqpmzxy");
    bodies
        .write_body(&id, None, "harissa", "body\n", "new")
        .await
        .unwrap();
    bodies
        .write_body(&id, Some("harissa"), "harissa-recipe", "body\n", "edit")
        .await
        .unwrap();

    assert!(root.join("zkqpmzxy harissa-recipe.viri").is_file());
    assert!(!root.join("zkqpmzxy harissa.viri").exists());
    assert_eq!(
        log(&bodies).await,
        vec![
            "",
            "viri: rename harissa to harissa-recipe",
            "viri: new harissa"
        ]
    );
}

#[tokio::test]
async fn hand_edits_stay_in_the_working_copy() {
    let (dir, mut bodies) = garden().await;
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
    let id = nid("zkqpmzxy");

    // An uncommitted hand edit to another file, made after the last snapshot.
    bodies
        .write_body(&id, None, "harissa", "viri body\n", "new")
        .await
        .unwrap();
    fs_err::write(root.join("sesame noodles.md"), "hand edit\n").unwrap();

    let other = nid("tsrqponm");
    bodies
        .write_body(&other, None, "sesame-noodles", "noodles\n", "new")
        .await
        .unwrap();

    assert_eq!(
        fs_err::read_to_string(root.join("sesame noodles.md")).unwrap(),
        "hand edit\n"
    );
    assert_eq!(
        log(&bodies).await,
        vec!["", "viri: new sesame-noodles", "viri: new harissa"]
    );
}

#[tokio::test]
async fn a_hand_edit_to_the_same_body_conflicts_in_the_working_copy() {
    let (dir, mut bodies) = garden().await;
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
    let id = nid("zkqpmzxy");

    bodies
        .write_body(&id, None, "harissa", "viri body\n", "new")
        .await
        .unwrap();

    // A hand edit to the same body file, unsnapshotted.
    fs_err::write(root.join("zkqpmzxy harissa.viri"), "hand body\n").unwrap();

    bodies
        .write_body(&id, Some("harissa"), "harissa", "new viri body\n", "edit")
        .await
        .unwrap();

    let on_disk = fs_err::read_to_string(root.join("zkqpmzxy harissa.viri")).unwrap();
    assert!(
        on_disk.contains("++++++"),
        "expected conflict markers on disk, got:\n{on_disk}"
    );
    assert_eq!(
        log(&bodies).await,
        vec!["", "viri: edit harissa", "viri: new harissa"]
    );
}

#[tokio::test]
async fn reads_a_missing_body_as_empty() {
    let (_dir, bodies) = garden().await;
    assert_eq!(bodies.read_body(&nid("zkqpmzxy")).unwrap(), "");
}
