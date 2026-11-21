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
        format!("#!/bin/sh\nsed -i.bak 's/^slug:$/slug: {slug}/' \"$1\" && rm \"$1.bak\"\n"),
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
        "#!/bin/sh\nsed -i.bak 's/^slug: harissa$/slug: harissa-paste/' \"$1\" && rm \"$1.bak\"\n",
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

/// An "editor" that quits without touching the buffer.
fn noop_editor(dir: &TempDir) -> Utf8PathBuf {
    script(dir, "noop.sh", "#!/bin/sh\nexit 0\n")
}

fn script(dir: &TempDir, name: &str, body: &str) -> Utf8PathBuf {
    let path = dir.path().join(name);
    fs_err::write(&path, body).unwrap();
    let mut perms = fs_err::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs_err::set_permissions(&path, perms).unwrap();
    Utf8PathBuf::try_from(path).unwrap()
}

#[test]
fn edit_with_an_untouched_valid_buffer_is_a_noop() {
    let (dir, root) = garden();
    let meta_before = fs_err::read_to_string(root.join("viri.kdl")).unwrap();
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", noop_editor(&dir))
        .assert()
        .success();
    assert_eq!(
        fs_err::read_to_string(root.join("viri.kdl")).unwrap(),
        meta_before
    );
}

#[test]
fn edit_uses_editor_when_visual_is_unset() {
    let (dir, root) = garden();
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env_remove("VISUAL")
        .env("EDITOR", noop_editor(&dir))
        .assert()
        .success();
}

#[test]
fn a_missing_editor_program_fails_loudly() {
    let (_dir, root) = garden();
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", "/nonexistent-viri-editor")
        .assert()
        .failure()
        .stderr(predicates::str::contains("failed to start the editor"));
}

#[test]
fn new_reopens_the_editor_after_parse_errors() {
    let (dir, root) = garden();
    let editor = script(
        &dir,
        "break-then-fix.sh",
        concat!(
            "#!/bin/sh\n",
            "stamp=$(dirname \"$0\")/invoked\n",
            "if [ ! -f \"$stamp\" ]; then\n",
            "    touch \"$stamp\"\n",
            "    sed -i.bak 's/^slug:$/broken line without a colon/' \"$1\" && rm \"$1.bak\"\n",
            "else\n",
            "    sed -i.bak 's/^broken line without a colon$/slug: second-round/' \"$1\" && rm \"$1.bak\"\n",
            "fi\n",
        ),
    );
    cmd()
        .args(["new"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", &editor)
        .assert()
        .success();
    let meta = fs_err::read_to_string(root.join("viri.kdl")).unwrap();
    assert!(meta.contains("slug \"second-round\""), "{meta}");
}

#[test]
fn quitting_with_parse_errors_keeps_a_recovery_file() {
    let (dir, root) = garden();
    let editor = script(
        &dir,
        "break-only.sh",
        "#!/bin/sh\nsed -i.bak 's/^slug:$/broken line/' \"$1\" && rm \"$1.bak\"\n",
    );
    cmd()
        .args(["new"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", &editor)
        .assert()
        .failure()
        .stderr(predicates::str::contains("recovery"));
    let recovery = root.join(".viri").join("recovery");
    let saved_entry = fs_err::read_dir(&recovery)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let saved_path = Utf8PathBuf::try_from(saved_entry.path()).unwrap();
    let saved = fs_err::read_to_string(&saved_path).unwrap();
    assert!(saved.contains("broken line"), "{saved}");
}

#[test]
fn a_recovery_failure_reports_the_lost_buffer() {
    let (dir, root) = garden();
    fs_err::write(root.join(".viri"), "a file, not a directory").unwrap();
    cmd()
        .args(["new"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", noop_editor(&dir))
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "could not write the recovery file",
        ));
}

#[test]
fn the_gitignore_gain_appends_after_a_missing_newline() {
    let (dir, root) = garden();
    fs_err::write(root.join(".gitignore"), "target").unwrap();
    cmd()
        .args(["new"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", noop_editor(&dir))
        .assert()
        .failure();
    let gitignore = fs_err::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(gitignore, "target\n.viri/\n");
}

#[test]
fn ls_outside_a_garden_fails() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["ls"])
        .current_dir(dir.path())
        .env_remove("VISUAL")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no garden found"));
}

#[test]
fn a_garden_flag_without_viri_kdl_fails() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["ls"])
        .arg("--garden")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("no garden found"));
}

#[test]
fn an_editor_that_exits_nonzero_fails() {
    let (dir, root) = garden();
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", script(&dir, "fail.sh", "#!/bin/sh\nexit 1\n"))
        .assert()
        .failure()
        .stderr(predicates::str::contains("the editor exited with"));
}

#[test]
fn vi_is_the_last_resort_editor() {
    let (dir, root) = garden();
    let bin = dir.path().join("bin");
    fs_err::create_dir(&bin).unwrap();
    let fake_vi = script(&dir, "bin/vi", "#!/bin/sh\nexit 0\n");
    let _ = fake_vi;
    let path = format!(
        "{}:{}",
        bin.to_str().unwrap(),
        std::env::var("PATH").unwrap_or_default()
    );
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("PATH", path)
        .assert()
        .success();
}

#[test]
fn a_buffer_that_loses_its_fence_self_heals() {
    let (dir, root) = garden();
    let editor = script(
        &dir,
        "drop-fence.sh",
        concat!(
            "#!/bin/sh\n",
            "stamp=$(dirname \"$0\")/dropped\n",
            "if [ ! -f \"$stamp\" ]; then\n",
            "    touch \"$stamp\"\n",
            "    sed -i.bak '1d' \"$1\" && rm \"$1.bak\"\n",
            "fi\n",
        ),
    );
    let before = fs_err::read_to_string(root.join("viri.kdl")).unwrap();
    // Deleting the opening fence makes the buffer unparseable; inject
    // restores it, and the untouched second round saves the same node.
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", &editor)
        .assert()
        .success();
    assert_eq!(
        fs_err::read_to_string(root.join("viri.kdl")).unwrap(),
        before
    );
}

#[test]
fn the_gitignore_gain_appends_after_a_trailing_newline() {
    let (dir, root) = garden();
    fs_err::write(root.join(".gitignore"), "target\n").unwrap();
    cmd()
        .args(["new"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", noop_editor(&dir))
        .assert()
        .failure();
    let gitignore = fs_err::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(gitignore, "target\n.viri/\n");
}

#[test]
#[cfg_attr(target_os = "macos", ignore = "APFS rejects non-UTF-8 file names")]
fn a_non_utf8_working_directory_fails_cleanly() {
    use std::os::unix::ffi::OsStrExt as _;
    let dir = TempDir::new().unwrap();
    let weird = dir.path().join(std::ffi::OsStr::from_bytes(b"bad-\xff"));
    fs_err::create_dir(&weird).unwrap();
    // A garden sits at the non-UTF-8 path itself, so the walk finds it
    // but cannot express it.
    fs_err::write(weird.join("viri.kdl"), "garden \"x\"\n").unwrap();
    cmd()
        .args(["ls"])
        .current_dir(&weird)
        .assert()
        .failure()
        .stderr(predicates::str::contains("no garden found"));
}

#[test]
fn a_changed_but_invalid_buffer_reopens_and_recovers() {
    let (dir, root) = garden();
    let editor = script(
        &dir,
        "validate-fail.sh",
        concat!(
            "#!/bin/sh\n",
            "stamp=$(dirname \"$0\")/invalid\n",
            "if [ ! -f \"$stamp\" ]; then\n",
            "    touch \"$stamp\"\n",
            "    sed -i.bak 's/^slug: harissa$/slug: Not-Kebab/' \"$1\" && rm \"$1.bak\"\n",
            "fi\n",
        ),
    );
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", &editor)
        .assert()
        .failure()
        .stderr(predicates::str::contains("recovery"));
    let meta = fs_err::read_to_string(root.join("viri.kdl")).unwrap();
    assert!(meta.contains("slug \"harissa\""), "{meta}");
}

#[test]
fn empty_editor_variables_have_no_editor() {
    let (_dir, root) = garden();
    cmd()
        .args(["edit", "z"])
        .arg("--garden")
        .arg(&root)
        .env("VISUAL", "")
        .env("EDITOR", "")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no editor to run"));
}
