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

/// The binary, kept away from the real viri config.
fn cmd() -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("viri");
    cmd.env("XDG_CONFIG_HOME", "/nonexistent-viri-config");
    cmd
}

/// A garden named `name` in a fresh jj workspace, its `viri.kdl` holding
/// `nodes` and no bodies.
fn bare_garden(name: &str, nodes: &[String]) -> (TempDir, Utf8PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(dir.path().canonicalize().unwrap()).unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        Workspace::init_simple(&settings(), dir.path(), &WorkspaceNameBuf::from("default"))
            .await
            .unwrap();
    });
    fs_err::write(
        root.join("viri.kdl"),
        format!("garden \"{name}\"\n\n{}", nodes.join("\n")),
    )
    .unwrap();
    (dir, root)
}

fn node(id: &str, slug: &str) -> String {
    format!("node \"{id}\" {{\n    slug \"{slug}\"\n    created \"2026-11-21T10:00:00Z\"\n}}\n")
}

/// The `other` garden: a second `harissa` and a `pantry`.
fn other_garden() -> (TempDir, Utf8PathBuf) {
    bare_garden(
        "other",
        &[node("xkkkkkkk", "harissa"), node("lmnoprst", "pantry")],
    )
}

/// The binary with a viri config listing `roots`, followed by `extra`.
fn configured(roots: &[&Utf8PathBuf], extra: &str) -> (TempDir, assert_cmd::Command) {
    let dir = TempDir::new().unwrap();
    let viri = dir.path().join("viri");
    fs_err::create_dir(&viri).unwrap();
    let gardens: String = roots
        .iter()
        .map(|root| format!("garden \"{root}\"\n"))
        .collect();
    fs_err::write(viri.join("config.kdl"), format!("{gardens}{extra}")).unwrap();
    let mut cmd = cmd();
    cmd.env("XDG_CONFIG_HOME", dir.path());
    (dir, cmd)
}

#[test]
fn ls_spans_every_garden_and_qualifies_shared_slugs() {
    let (_a, test) = garden();
    let (_b, other) = other_garden();
    let (dir, mut viri) = configured(&[&test, &other], "");
    viri.args(["ls"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(concat!(
            "q  viridian\n",
            "z  test/harissa  #recipe #spicy\n",
            "x  other/harissa\n",
            "l  pantry\n",
        ));
}

#[test]
fn the_garden_flag_narrows_a_read_to_one_garden() {
    let (_a, test) = garden();
    let (_b, other) = other_garden();
    let (dir, mut viri) = configured(&[&test, &other], "");
    viri.args(["ls", "--garden", "other"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout("x  harissa\nl  pantry\n");
}

#[test]
fn show_and_edit_find_the_garden_holding_the_node() {
    let (_a, test) = garden();
    let (_b, other) = other_garden();
    let (dir, mut viri) = configured(&[&test, &other], "");
    viri.args(["show", "x"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("id: xkkkkkkk\nslug: harissa\n"));

    let (dir, mut viri) = configured(&[&test, &other], "");
    viri.args(["edit", "x"])
        .current_dir(dir.path())
        .env("VISUAL", body_editor(&dir, "Toast the cumin."))
        .assert()
        .success();
    let body = fs_err::read_to_string(other.join("xkkkkkkk harissa.viri")).unwrap();
    assert_eq!(body, "Toast the cumin.\n");
    assert!(!test.join("xkkkkkkk harissa.viri").exists());
}

#[test]
fn a_prefix_matching_nodes_in_two_gardens_is_ambiguous() {
    let (_a, test) = garden();
    let (_b, other) = bare_garden("other", &[node("zxxxxxxx", "zest")]);
    let (dir, mut viri) = configured(&[&test, &other], "");
    viri.args(["show", "z"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("matches several node ids"));
}

#[test]
fn a_node_id_in_two_gardens_fails() {
    let (_a, test) = garden();
    let (_b, other) = bare_garden("other", &[node("zkqpmzxy", "copy")]);
    let (dir, mut viri) = configured(&[&test, &other], "");
    viri.args(["ls"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("node zkqpmzxy is in two gardens"));
}

#[test]
fn an_unreachable_configured_garden_fails_reads() {
    let missing = Utf8PathBuf::from("/nonexistent-viri-garden");
    let (dir, mut viri) = configured(&[&missing], "");
    viri.args(["ls"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("can't reach the garden"));
}

#[test]
fn the_enclosing_garden_appears_once_when_configured_too() {
    let (_a, test) = garden();
    let (_dir, mut viri) = configured(&[&test], "");
    viri.args(["ls"])
        .current_dir(&test)
        .assert()
        .success()
        .stdout("q  viridian\nz  harissa  #recipe #spicy\n");
}

#[test]
fn new_prefers_the_enclosing_garden_over_the_default() {
    let (_a, test) = garden();
    let (_b, other) = other_garden();
    let (dir, mut viri) = configured(&[&test, &other], "default \"other\"\n");
    viri.args(["new"])
        .current_dir(&test)
        .env("VISUAL", slug_editor(&dir, "sesame-noodles"))
        .assert()
        .success();
    let meta = fs_err::read_to_string(test.join("viri.kdl")).unwrap();
    assert!(meta.contains("slug \"sesame-noodles\""), "{meta}");
}

/// Runs `viri new` outside any garden with the `other` garden, plus `test` when `both`, and
/// `extra`, then returns what happened and the `other` garden's metadata.
fn new_outside(extra: &str, args: &[&str], both: bool) -> (assert_cmd::assert::Assert, String) {
    let (_a, test) = garden();
    let (_b, other) = other_garden();
    let roots: Vec<&Utf8PathBuf> = if both {
        vec![&test, &other]
    } else {
        vec![&other]
    };
    let (dir, mut viri) = configured(&roots, extra);
    let assert = viri
        .arg("new")
        .args(args)
        .current_dir(dir.path())
        .env("VISUAL", slug_editor(&dir, "sesame-noodles"))
        .assert();
    let meta = fs_err::read_to_string(other.join("viri.kdl")).unwrap();
    (assert, meta)
}

#[test]
fn new_uses_the_default_outside_any_garden() {
    let (assert, meta) = new_outside("default \"other\"\n", &[], true);
    assert.success();
    assert!(meta.contains("slug \"sesame-noodles\""), "{meta}");
}

#[test]
fn new_uses_a_named_garden() {
    let (assert, meta) = new_outside("", &["-g", "other"], true);
    assert.success();
    assert!(meta.contains("slug \"sesame-noodles\""), "{meta}");
}

#[test]
fn new_uses_the_only_known_garden() {
    let (assert, meta) = new_outside("", &[], false);
    assert.success();
    assert!(meta.contains("slug \"sesame-noodles\""), "{meta}");
}

#[test]
fn new_asks_which_garden_when_several_could_take_it() {
    let (assert, meta) = new_outside("", &[], true);
    assert.failure().stderr(predicates::str::contains(
        "which garden should the new node go in",
    ));
    assert!(!meta.contains("sesame-noodles"), "{meta}");
}

#[test]
fn new_outside_any_garden_with_no_config_fails() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["new"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("no garden found"));
}

#[test]
fn an_unknown_garden_name_lists_the_known_ones() {
    let (_b, other) = other_garden();
    let (dir, mut viri) = configured(&[&other], "");
    viri.args(["ls", "-g", "work"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("no garden is named `work`"))
        .stderr(predicates::str::contains("named other"));
}

#[test]
fn an_unknown_default_fails() {
    let (assert, _meta) = new_outside("default \"work\"\n", &[], true);
    assert
        .failure()
        .stderr(predicates::str::contains("no garden is named `work`"));
}

#[test]
fn a_broken_config_fails_every_command() {
    let (_a, test) = garden();
    let (dir, mut viri) = configured(&[], "garden \"relative\"\n");
    viri.args(["ls"])
        .arg("--garden")
        .arg(&test)
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("is relative"));
}

#[test]
fn the_config_falls_back_to_home() {
    let (_garden_dir, root) = garden();
    let home = TempDir::new().unwrap();
    let config = home.path().join(".config").join("viri");
    fs_err::create_dir_all(&config).unwrap();
    // Exercises `~/` expansion against $HOME, too.
    let name = root.file_name().unwrap();
    fs_err::rename(&root, home.path().join(name)).unwrap();
    fs_err::write(config.join("config.kdl"), format!("garden \"~/{name}\"\n")).unwrap();
    cmd()
        .args(["ls"])
        .current_dir(home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", home.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("harissa"));
}

#[test]
fn a_non_utf8_xdg_config_home_fails() {
    use std::os::unix::ffi::OsStrExt as _;
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["ls"])
        .current_dir(dir.path())
        .env("XDG_CONFIG_HOME", std::ffi::OsStr::from_bytes(b"/bad-\xff"))
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "$XDG_CONFIG_HOME is not valid UTF-8",
        ));
}

#[test]
fn a_garden_value_with_a_slash_is_a_path() {
    let (_garden_dir, root) = garden();
    let parent = root.parent().unwrap();
    let name = root.file_name().unwrap();
    cmd()
        .args(["ls", "--garden", &format!("./{name}")])
        .current_dir(parent)
        .assert()
        .success()
        .stdout(predicates::str::contains("harissa"));
    cmd()
        .args(["ls", "--garden", "."])
        .current_dir(&root)
        .assert()
        .success();
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

#[test]
fn show_rejects_a_malformed_prefix() {
    let (_dir, root) = garden();
    cmd()
        .args(["show", "ZK"])
        .arg("--garden")
        .arg(&root)
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid value 'ZK'"))
        .stderr(predicates::str::contains("invalid node id prefix"));
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
        .stderr(predicates::str::contains("has no viri.kdl"));
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

#[test]
fn skill_prints_the_guide_outside_a_garden() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["__internal", "skill"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(viri::cli::SKILL_GUIDE);
}

#[test]
fn the_skill_guide_covers_every_command() {
    use clap::CommandFactory;
    let cli = viri::cli::Cli::command();
    for sub in cli.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        let usage = format!("`viri {}", sub.get_name());
        assert!(
            viri::cli::SKILL_GUIDE.contains(&usage),
            "the guide never shows {usage}`"
        );
    }
}

#[test]
fn the_internal_command_is_hidden_from_help() {
    use predicates::boolean::PredicateBooleanExt;
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("__internal").not());
}
