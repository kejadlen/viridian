//! The garden's bodies: `<change-id> <slug>.viri` files in a jj repo.
//!
//! viri commits bodies through jj-lib so its commits and hand edits share one
//! working copy. Each save snapshots the working copy, builds the new commit
//! from `@`'s parents' tree, and rebases `@` onto it. The working copy keeps
//! any uncommitted hand edits, and a hand edit touching the same body leaves
//! a conflict in `@`.

use std::sync::Arc;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use futures::AsyncReadExt;
use futures::future::try_join_all;
use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::backend::CopyId;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigLoadError;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::default_backend_factories::default_backend_factories;
use jj_lib::default_backend_factories::default_working_copy_factories;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::op_store::OpStoreError;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::repo::RepoLoaderError;
use jj_lib::repo::RewriteRootCommit;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::rewrite::rebase_commit;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use jj_lib::transaction::TransactionCommitError;
use jj_lib::working_copy::CheckoutError;
use jj_lib::working_copy::SnapshotError;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::WorkingCopyFreshness;
use jj_lib::working_copy::WorkingCopyStateError;
use jj_lib::workspace::DefaultWorkspaceLoaderFactory;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceLoaderFactory as _;
use miette::Diagnostic;
use thiserror::Error;

use crate::id::NodeId;

/// The name of a node's body file: `<change-id> <slug>.viri`.
///
/// viri finds a body by splitting the filename on the first space, so the
/// slug in a filename is cosmetic.
pub fn body_file_name(id: &NodeId, slug: &str) -> String {
    format!("{id} {slug}.viri")
}

/// The body store for one garden: its jj workspace plus the loaded repo.
pub struct Bodies {
    root: Utf8PathBuf,
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
}

impl Bodies {
    pub async fn open(root: &Utf8Path) -> Result<Bodies, BodiesError> {
        let settings = Arc::new(load_settings()?);
        Bodies::open_with(settings, root).await
    }

    /// Opens the body store with explicit settings, for embedders and tests.
    pub async fn open_with(
        settings: Arc<UserSettings>,
        root: &Utf8Path,
    ) -> Result<Bodies, BodiesError> {
        let loader = DefaultWorkspaceLoaderFactory
            .create(root.as_std_path())
            .map_err(|err| BodiesError::Load(Box::new(err)))?;
        let workspace = loader
            .load(
                &settings,
                &default_backend_factories(),
                &default_working_copy_factories(),
            )
            .map_err(|err| BodiesError::Load(Box::new(err)))?;
        let repo = workspace
            .repo_loader()
            .load_at_head(Some(workspace.workspace_name()))
            .await?;
        Ok(Bodies {
            root: root.to_owned(),
            workspace,
            repo,
        })
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// Reads a node's body from the working copy, or an empty string when the
    /// body file doesn't exist yet.
    pub fn read_body(&self, id: &NodeId) -> Result<String, BodiesError> {
        let mut paths = body_paths(&self.root, id)?;
        match paths.pop() {
            None => Ok(String::new()),
            Some(path) if paths.is_empty() => Ok(fs_err::read_to_string(&path)
                .map_err(|err| BodiesError::Read { path, source: err })?),
            Some(_) => Err(BodiesError::DuplicateBody { id: *id, paths }),
        }
    }

    /// Commits a node's body below `@`, as `jj new -B @` would.
    ///
    /// A stale slug gets a rename commit of its own, before the commit that
    /// changes the content. Nothing is committed when neither the file name
    /// nor the content changed.
    pub async fn write_body(
        &mut self,
        id: &NodeId,
        old_slug: Option<&str>,
        new_slug: &str,
        content: &str,
        verb: &str,
    ) -> Result<(), BodiesError> {
        self.snapshot().await?;

        let store = self.repo.store().clone();
        let workspace_name = self.workspace.workspace_name().to_owned();
        let wc_commit = wc_commit(&self.repo, &workspace_name).await?;
        let parent_commits: Vec<Commit> = try_join_all(
            wc_commit
                .parent_ids()
                .iter()
                .map(|id| store.get_commit_async(id)),
        )
        .await?;
        let old_wc_tree = wc_commit.tree();
        let mut tree = merge_commit_trees(self.repo.as_ref(), &parent_commits).await?;

        let mut tx = self.repo.start_transaction();
        let mut parents: Vec<CommitId> = wc_commit.parent_ids().to_vec();

        if let Some(old_slug) = old_slug
            && old_slug != new_slug
        {
            let old_path = body_repo_path(id, old_slug);
            let value = tree.path_value(&old_path).await?;
            if let Some(file) = value.into_resolved().ok().flatten() {
                let new_path = body_repo_path(id, new_slug);
                let mut builder = MergedTreeBuilder::new(tree.clone());
                builder.set_or_remove(old_path, None);
                builder.set_or_remove(new_path, Some(file));
                tree = builder.write_tree().await?;
                detached_commit(
                    &mut tx,
                    &mut parents,
                    tree.clone(),
                    format!("viri: rename {old_slug} to {new_slug}"),
                )
                .await?;
            }
        }

        let new_path = body_repo_path(id, new_slug);
        let current = file_content(&store, &tree, &new_path).await?;
        if current.as_deref() != Some(content.as_bytes()) {
            let file_id = store.write_file(&new_path, &mut content.as_bytes()).await?;
            let mut builder = MergedTreeBuilder::new(tree.clone());
            builder.set_or_remove(
                new_path,
                Some(TreeValue::File {
                    id: file_id,
                    executable: false,
                    copy_id: CopyId::placeholder(),
                }),
            );
            tree = builder.write_tree().await?;
            let description = format!("viri: {verb} {new_slug}");
            detached_commit(&mut tx, &mut parents, tree.clone(), description.clone()).await?;
        }

        if parents == wc_commit.parent_ids() {
            // Neither the file name nor the content changed.
            return Ok(());
        }

        rebase_commit(tx.repo_mut(), wc_commit.clone(), parents).await?;
        tx.repo_mut().rebase_descendants().await?;
        let repo = tx.commit("viri: write body").await?;

        let new_wc_id = repo
            .view()
            .get_wc_commit_id(&workspace_name)
            .ok_or(BodiesError::NoWorkingCopy)?
            .clone();
        let new_wc = self.repo.store().get_commit_async(&new_wc_id).await?;
        self.workspace
            .check_out(repo.op_id().clone(), Some(&old_wc_tree), &new_wc)
            .await?;
        self.repo = repo;
        Ok(())
    }

    /// Snapshots the working copy into `@`, so hand edits made with jj are
    /// part of `@` before viri inserts commits below it.
    async fn snapshot(&mut self) -> Result<(), BodiesError> {
        let workspace_name = self.workspace.workspace_name().to_owned();
        let mut repo = self.repo.clone();
        let mut locked_ws = self.workspace.start_working_copy_mutation().await?;
        let mut wc_commit = wc_commit(&repo, &workspace_name).await?;
        match WorkingCopyFreshness::check_stale(locked_ws.locked_wc(), &wc_commit, &repo).await? {
            WorkingCopyFreshness::Fresh => {}
            WorkingCopyFreshness::Updated(op) => {
                // The working copy moved past our loaded head; the commit to
                // rewrite is the reloaded repo's, not the stale one's.
                repo = repo.reload_at(&op).await?;
                wc_commit = crate::body::wc_commit(&repo, &workspace_name).await?;
            }
            WorkingCopyFreshness::WorkingCopyStale | WorkingCopyFreshness::SiblingOperation => {
                return Err(BodiesError::Stale);
            }
        }

        let options = SnapshotOptions {
            base_ignores: GitIgnoreFile::empty(),
            progress: None,
            start_tracking_matcher: &EverythingMatcher,
            force_tracking_matcher: &NothingMatcher,
            max_new_file_size: u64::MAX,
        };
        let (new_tree, _stats) = locked_ws.locked_wc().snapshot(&options).await?;
        self.repo = repo;
        if new_tree.tree_ids_and_labels() == wc_commit.tree().tree_ids_and_labels() {
            return Ok(());
        }

        let repo = self.repo.clone();
        let mut tx = repo.start_transaction();
        let new_wc = tx
            .repo_mut()
            .rewrite_commit(&wc_commit)
            .set_tree(new_tree)
            .write()
            .await?;
        tx.repo_mut()
            .set_wc_commit(workspace_name.clone(), new_wc.id().clone())?;
        tx.repo_mut().rebase_descendants().await?;
        let repo = tx.commit("viri: snapshot working copy").await?;
        locked_ws.finish(repo.op_id().clone()).await?;
        self.repo = repo;
        Ok(())
    }
}

fn body_paths(root: &Utf8Path, id: &NodeId) -> Result<Vec<Utf8PathBuf>, BodiesError> {
    let prefix = format!("{id} ");
    let mut paths = Vec::new();
    for entry in fs_err::read_dir(root)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            // cov-excl-macos-start
            paths.push(Utf8PathBuf::try_from(entry.path()).map_err(|err| {
                BodiesError::Load(format!("body file name is not UTF-8: {}", err).into())
            })?);
            // cov-excl-macos-stop
        }
    }
    paths.sort();
    Ok(paths)
}

fn body_repo_path(id: &NodeId, slug: &str) -> RepoPathBuf {
    // Body file names are plain file names — not absolute, no parent
    // components — so from_internal_string cannot reject one.
    #[allow(clippy::expect_used)]
    RepoPathBuf::from_internal_string(body_file_name(id, slug))
        .expect("body file name is a valid repo path")
}

async fn file_content(
    store: &Arc<Store>,
    tree: &MergedTree,
    path: &RepoPathBuf,
) -> Result<Option<Vec<u8>>, BackendError> {
    let value = tree.path_value(path).await?;
    // A conflicted path counts as absent: the write replaces the conflict.
    let Some(TreeValue::File { id, .. }) = value.into_resolved().ok().flatten() else {
        return Ok(None);
    };
    let mut reader = store.read_file(path, &id).await?;
    let mut content = Vec::new();
    reader
        .read_to_end(&mut content)
        .await
        // The store just handed us this reader; an I/O failure mid-read has
        // no reproduction path in tests.
        .map_err(|err| BackendError::Other(Box::new(err)))?; // cov-excl-line
    Ok(Some(content))
}
async fn detached_commit(
    tx: &mut Transaction,
    parents: &mut Vec<CommitId>,
    tree: MergedTree,
    description: String,
) -> Result<Commit, BackendError> {
    let commit = tx
        .repo_mut()
        .new_commit(std::mem::take(parents), tree)
        .detach();
    let mut commit = commit;
    commit.set_description(description);
    let written = commit.write(tx.repo_mut()).await?;
    *parents = vec![written.id().clone()];
    Ok(written)
}

async fn wc_commit(
    repo: &Arc<ReadonlyRepo>,
    workspace_name: &WorkspaceName,
) -> Result<Commit, BodiesError> {
    let Some(id) = repo.view().get_wc_commit_id(workspace_name) else {
        return Err(BodiesError::NoWorkingCopy);
    };
    Ok(repo.store().get_commit_async(id).await?)
}

/// Builds user settings from jj's defaults, the environment, and jj's user
/// config files, so viri's commits carry the same identity as jj's.
fn load_settings() -> Result<UserSettings, BodiesError> {
    let mut config = StackedConfig::with_defaults();

    let mut env = ConfigLayer::empty(ConfigSource::EnvBase);
    // Fixed constant names cannot collide with existing values.
    #[allow(clippy::expect_used)]
    {
        env.set_value("operation.hostname", hostname())
            .expect("constant config name is valid");
        env.set_value("operation.username", username())
            .expect("constant config name is valid");
    }
    config.add_layer(env);

    for path in user_config_paths() {
        if path.exists() {
            let layer = ConfigLayer::load_from_file(ConfigSource::User, path.into_std_path_buf())?;
            config.add_layer(layer);
        }
    }

    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    UserSettings::from_config_and_home_dir(config, home).map_err(BodiesError::ConfigGet)
}

fn hostname() -> String {
    fs_err::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_owned()) // cov-excl-macos-line
        .filter(|h| !h.is_empty()) // cov-excl-macos-line
        // Linux always serves /proc/sys/kernel/hostname; the fallbacks are
        // for platforms without it.
        .or_else(|| std::env::var("HOSTNAME").ok()) // cov-excl-line
        .unwrap_or_else(|| "localhost".to_owned()) // cov-excl-line
}

fn username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn user_config_paths() -> Vec<Utf8PathBuf> {
    let config_dir = || -> Option<Utf8PathBuf> {
        let raw = std::env::var_os("JJ_CONFIG_DIR")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(|base| std::path::PathBuf::from(base).join("jj"))
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".config").join("jj"))
            });
        raw.and_then(|p| Utf8PathBuf::try_from(p).ok())
    };
    if let Ok(path) = std::env::var("JJ_CONFIG") {
        return vec![Utf8PathBuf::from(path)];
    }
    let Some(dir) = config_dir() else {
        return vec![];
    };
    let mut paths = vec![dir.join("config.toml")];
    let mut confd: Vec<Utf8PathBuf> = match fs_err::read_dir(dir.join("conf.d")) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|e| Utf8PathBuf::try_from(e.path()).ok())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect(),
        Err(_) => vec![],
    };
    confd.sort();
    paths.extend(confd);
    paths
}

#[derive(Error, Diagnostic, Debug)]
pub enum BodiesError {
    #[error("failed to load the garden's jj repo")]
    #[diagnostic(code(viri_core::body::load))]
    Load(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("failed to read {}", .path)]
    #[diagnostic(code(viri_core::body::read))]
    Read {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("node {id} has several body files: {}", .paths.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "))]
    #[diagnostic(
        code(viri_core::body::duplicate),
        help("two body files with the same change id are an error; remove the stale one")
    )]
    DuplicateBody { id: NodeId, paths: Vec<Utf8PathBuf> },

    #[error("the garden's working copy is behind the repo; run `jj st` to bring it up to date")]
    #[diagnostic(code(viri_core::body::stale))]
    Stale,

    #[error("the garden's jj repo has no working copy for this workspace")]
    #[diagnostic(code(viri_core::body::no_working_copy))]
    NoWorkingCopy,

    #[error("failed to load jj config")]
    #[diagnostic(code(viri_core::body::config))]
    Config(#[from] ConfigLoadError),

    #[error("failed to read a jj config value")]
    #[diagnostic(code(viri_core::body::config_value))]
    ConfigGet(#[from] ConfigGetError),

    #[error("failed to update the garden's working copy")]
    #[diagnostic(code(viri_core::body::working_copy))]
    WorkingCopy(#[from] WorkingCopyStateError),

    #[error("failed to snapshot the garden's working copy")]
    #[diagnostic(code(viri_core::body::snapshot))]
    Snapshot(#[from] SnapshotError),

    #[error("failed to check out the working copy after saving")]
    #[diagnostic(code(viri_core::body::checkout))]
    Checkout(#[from] CheckoutError),

    #[error("the jj repo is inconsistent: {0}")]
    #[diagnostic(code(viri_core::body::backend))]
    Backend(#[from] BackendError),

    #[error("failed to load or reload the garden's repo")]
    #[diagnostic(code(viri_core::body::repo))]
    Repo(#[from] RepoLoaderError),

    #[error("failed to read repo history")]
    #[diagnostic(code(viri_core::body::op_store))]
    OpStore(#[from] OpStoreError),

    #[error("failed to commit viri's changes")]
    #[diagnostic(code(viri_core::body::transaction))]
    Transaction(#[from] TransactionCommitError),

    #[error("refused to rewrite the root commit")]
    #[diagnostic(code(viri_core::body::rewrite_root))]
    RewriteRoot(#[from] RewriteRootCommit),

    #[error(transparent)]
    #[diagnostic(code(viri_core::body::io))]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt as _;

    // These tests mutate process environment; the mutex keeps them from
    // racing each other.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn set_env(key: &str, value: Option<&str>) {
        // Safe under the ENV mutex, within a single-threaded test run.
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn jj_config_replaces_the_config_directory() {
        let _guard = ENV.lock().unwrap();
        set_env("JJ_CONFIG", Some("/tmp/one-config.toml"));
        assert_eq!(
            user_config_paths(),
            vec![Utf8PathBuf::from("/tmp/one-config.toml")]
        );
        set_env("JJ_CONFIG", None);
    }

    #[test]
    fn config_paths_follow_the_config_directory() {
        let _guard = ENV.lock().unwrap();
        set_env("JJ_CONFIG", None);
        set_env("JJ_CONFIG_DIR", Some("/tmp/jj-config"));
        assert_eq!(
            user_config_paths(),
            vec![Utf8PathBuf::from("/tmp/jj-config/config.toml")]
        );
        set_env("JJ_CONFIG_DIR", None);

        set_env("XDG_CONFIG_HOME", Some("/tmp/xdg"));
        assert_eq!(
            user_config_paths(),
            vec![Utf8PathBuf::from("/tmp/xdg/jj/config.toml")]
        );
        set_env("XDG_CONFIG_HOME", None);
    }

    #[test]
    fn a_present_config_file_is_loaded() {
        let _guard = ENV.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        fs_err::write(
            dir.path().join("config.toml"),
            "[user]\nname = \"Loaded\"\nemail = \"l@example.com\"\n",
        )
        .unwrap();
        set_env(
            "JJ_CONFIG",
            Some(dir.path().join("config.toml").to_str().unwrap()),
        );
        let settings = load_settings().unwrap();
        assert_eq!(settings.user_name(), "Loaded");

        // A config directory with no config.toml skips it, and the
        // defaults leave the identity empty rather than erroring.
        let empty = tempfile::TempDir::new().unwrap();
        set_env("JJ_CONFIG", None);
        set_env("JJ_CONFIG_DIR", Some(empty.path().to_str().unwrap()));
        assert_eq!(load_settings().unwrap().user_name(), "");
        set_env("JJ_CONFIG_DIR", None);
    }

    #[test]
    fn without_any_config_environment_there_are_no_paths() {
        let _guard = ENV.lock().unwrap();
        set_env("JJ_CONFIG", None);
        set_env("JJ_CONFIG_DIR", None);
        set_env("XDG_CONFIG_HOME", None);
        set_env("HOME", None);
        assert!(user_config_paths().is_empty());
    }

    #[test]
    fn confd_toml_files_are_read_sorted_and_others_skipped() {
        let _guard = ENV.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let confd = dir.path().join("conf.d");
        fs_err::create_dir(&confd).unwrap();
        fs_err::write(confd.join("b.toml"), "").unwrap();
        fs_err::write(confd.join("a.toml"), "").unwrap();
        fs_err::write(confd.join("ignored.txt"), "").unwrap();
        set_env("JJ_CONFIG_DIR", Some(dir.path().to_str().unwrap()));

        let paths = user_config_paths();
        let confd_paths: Vec<String> = paths[1..]
            .iter()
            .map(|p| p.file_name().unwrap().to_string())
            .collect();
        assert_eq!(confd_paths, vec!["a.toml", "b.toml"]);

        set_env("JJ_CONFIG_DIR", None);
    }

    // cov-excl-macos-start
    #[test]
    #[cfg_attr(target_os = "macos", ignore = "APFS rejects non-UTF-8 file names")]
    fn non_utf8_conf_entries_are_skipped() {
        let _guard = ENV.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let confd = dir.path().join("conf.d");
        fs_err::create_dir(&confd).unwrap();
        let bad = std::ffi::OsStr::from_bytes(b"bad-\xff.toml");
        fs_err::write(confd.join(bad), "").unwrap();
        fs_err::write(confd.join("good.toml"), "").unwrap();
        set_env("JJ_CONFIG_DIR", Some(dir.path().to_str().unwrap()));

        assert_eq!(
            user_config_paths()[1..],
            vec![Utf8PathBuf::try_from(confd.join("good.toml")).unwrap()]
        );
        set_env("JJ_CONFIG_DIR", None);
    }
    // cov-excl-macos-stop

    #[test]
    fn hostname_and_username_fall_back() {
        assert!(!hostname().is_empty());
        assert!(!username().is_empty());
    }

    #[test]
    fn username_falls_back_to_logname_then_unknown() {
        let _guard = ENV.lock().unwrap();
        let user = std::env::var("USER").ok();
        let logname = std::env::var("LOGNAME").ok();
        set_env("USER", None);
        set_env("LOGNAME", Some("logged-in"));
        assert_eq!(username(), "logged-in");
        set_env("LOGNAME", None);
        assert_eq!(username(), "unknown");
        set_env("USER", user.as_deref());
        set_env("LOGNAME", logname.as_deref());
    }
}
