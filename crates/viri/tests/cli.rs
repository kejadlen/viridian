//! End-to-end tests for the `viri` binary against real gardens.

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

use std::os::unix::fs::PermissionsExt;

use assert_cmd::cargo::cargo_bin_cmd;
use camino::Utf8PathBuf;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::Workspace;
use tempfile::TempDir;

fn cmd() -> assert_cmd::Command {
    cargo_bin_cmd!("viri")
}

fn settings() -> std::sync::Arc<UserSettings> {
    let mut config = StackedConfig::with_defaults();
    let mut user = ConfigLayer::empty(ConfigSource::User);
    user.set_value("user.name", "Test").unwrap();
    user.set_value("user.email", "test@example.com").unwrap();
    user.set_value("operation.hostname", "test").unwrap();
    user.set_value("operation.username", "test").unwrap();
    config.add_layer(user);
    std::sync::Arc::new(UserSettings::from_config_and_home_dir(config, None).unwrap())
}

/// A garden with one node (`harissa`, tagged `recipe`, parented on the
/// design note) and a body.
fn garden() -> (TempDir, Utf8PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Workspace::init_simple(&settings(), dir.path(), &WorkspaceNameBuf::from("default"))
            .await
            .unwrap();
    });

    fs_err::write(
        root.join("viri.kdl"),
        concat!(
            "garden \"test\"\n",
            "\n",
            "node \"qmplyuym\" {\n",
            "    slug \"viridian\"\n",
            "    created \"2026-11-20T14:46:41Z\"\n",
            "}\n",
            "\n",
            "node \"zkqpmzxy\" {\n",
            "    slug \"harissa\"\n",
            "    created \"2026-11-21T10:00:00Z\"\n",
            "    tags \"recipe\"\n",
            "    tags \"spicy\"\n",
            "    parent \"qmplyuym\"\n",
            "}\n",
        ),
    )
    .unwrap();

    // Seed the body the way viri itself does: committed below @.
    let id: viri_core::NodeId = "zkqpmzxy".parse().unwrap();
    let bodies = runtime.block_on(async {
        let mut bodies = viri_core::body::Bodies::open_with(settings(), &root)
            .await
            .unwrap();
        bodies
            .write_body(&id, None, "harissa", "Blend the chilis.\n", "new")
            .await
            .unwrap();
        bodies
    });
    drop(bodies);
    (dir, root)
}

/// An "editor" that replaces the empty `slug:` line with a real slug.
fn slug_editor(dir: &TempDir, slug: &str) -> Utf8PathBuf {
    let path = dir.path().join("fill-slug.sh");
    fs_err::write(
        &path,
        format!("#!/bin/sh\nsed -i 's/^slug:$/slug: {slug}/' \"$1\"\n"),
    )
    .unwrap();
    let mut perms = fs_err::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs_err::set_permissions(&path, perms).unwrap();
    Utf8PathBuf::try_from(path).unwrap()
}

/// An "editor" that appends a line to the body.
fn body_editor(dir: &TempDir, line: &str) -> Utf8PathBuf {
    let path = dir.path().join("edit-body.sh");
    fs_err::write(
        &path,
        format!("#!/bin/sh\nprintf '%s\\n' '{line}' >> \"$1\"\n"),
    )
    .unwrap();
    let mut perms = fs_err::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs_err::set_permissions(&path, perms).unwrap();
    Utf8PathBuf::try_from(path).unwrap()
}

#[test]
fn ls_lists_short_ids_slugs_and_tags() {
    let (_dir, root) = garden();
    cmd()
        .args(["ls"])
        .arg("--garden")
        .arg(&root)
        .assert()
        .success()
        .stdout("q  viridian\nz  harissa  #recipe #spicy\n");
}

#[test]
fn show_prints_the_edit_buffer() {
    let (_dir, root) = garden();
    cmd()
        .args(["show", "z"])
        .arg("--garden")
        .arg(&root)
        .assert()
        .success()
        .stdout(predicates::str::contains("id: zkqpmzxy\nslug: harissa\n"))
        .stdout(predicates::str::contains("  - recipe\n"))
        .stdout(predicates::str::contains("  - qmplyuym\n"))
        .stdout(predicates::str::contains("Blend the chilis.\n"));
}

#[test]
fn new_creates_a_node_with_body_and_metadata() {
    let (dir, root) = garden();
    cmd()
        .args(["new", "z"])
        .arg("--garden")
        .arg(&root)
        .env("EDITOR", slug_editor(&dir, "sesame-noodles"))
        .env("VISUAL", slug_editor(&dir, "sesame-noodles"))
        .assert()
        .success();

    let meta = fs_err::read_to_string(root.join("viri.kdl")).unwrap();
    assert!(meta.contains("slug \"sesame-noodles\""), "{meta}");
    assert!(meta.contains("parent \"zkqpmzxy\""), "{meta}");
    let bodies: Vec<_> = fs_err::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("sesame") || name.contains("sesame"))
        .collect();
    assert_eq!(bodies.len(), 1, "{bodies:?}");
}

#[test]
fn new_without_a_slug_keeps_a_recovery_file_and_changes_nothing() {
    let (dir, root) = garden();
    let meta_before = fs_err::read_to_string(root.join("viri.kdl")).unwrap();

    let no_op = dir.path().join("noop.sh");
    fs_err::write(&no_op, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs_err::metadata(&no_op).unwrap().permissions();
    perms.set_mode(0o755);
    fs_err::set_permissions(&no_op, perms).unwrap();

    cmd()
        .args(["new"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", &no_op)
        .assert()
        .failure()
        .stderr(predicates::str::contains("recovery"));

    let meta_after = fs_err::read_to_string(root.join("viri.kdl")).unwrap();
    assert_eq!(meta_before, meta_after);
    let recovery = root.join(".viri").join("recovery");
    assert_eq!(fs_err::read_dir(&recovery).unwrap().count(), 1);
    let gitignore = fs_err::read_to_string(root.join(".gitignore")).unwrap();
    assert!(gitignore.contains(".viri/"));
}

#[test]
fn edit_renames_a_stale_slug() {
    let (dir, root) = garden();
    let rename = dir.path().join("rename.sh");
    fs_err::write(
        &rename,
        "#!/bin/sh\nsed -i 's/^slug: harissa$/slug: harissa-paste/' \"$1\"\n",
    )
    .unwrap();
    let mut perms = fs_err::metadata(&rename).unwrap().permissions();
    perms.set_mode(0o755);
    fs_err::set_permissions(&rename, perms).unwrap();

    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", &rename)
        .assert()
        .success();

    assert!(root.join("zkqpmzxy harissa-paste.viri").is_file());
    assert!(!root.join("zkqpmzxy harissa.viri").exists());
    let meta = fs_err::read_to_string(root.join("viri.kdl")).unwrap();
    assert!(meta.contains("slug \"harissa-paste\""), "{meta}");
}

#[test]
fn edit_appends_to_the_body() {
    let (dir, root) = garden();
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", body_editor(&dir, "Toast the cumin."))
        .assert()
        .success();

    let body = fs_err::read_to_string(root.join("zkqpmzxy harissa.viri")).unwrap();
    assert!(body.contains("Blend the chilis."));
    assert!(body.contains("Toast the cumin."));
}

#[test]
fn show_reports_an_unknown_prefix() {
    let (_dir, root) = garden();
    cmd()
        .args(["show", "w"])
        .arg("--garden")
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicates::str::contains("no node id starts with `w`"));
}
