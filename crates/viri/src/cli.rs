//! The `viri` command line: new, edit, ls, and show.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::convert::Infallible;
use std::io::IsTerminal as _;
use std::str::FromStr;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use clap::Parser;
use clap::Subcommand;
use miette::Diagnostic;
use thiserror::Error;
use viri_core::Garden;
use viri_core::Gardens;
use viri_core::Node;
use viri_core::NodeId;
use viri_core::id;
use viri_core::id::Prefix;
use viri_core::schema::NodeMeta;

use crate::buffer::Buffer;
use crate::config::Config;
use crate::editor;
use crate::prompt;

#[derive(Parser)]
#[command(name = "viri", about = "Tend a viridian garden", version = env!("VIRI_VERSION"))]
pub struct Cli {
    /// A garden name from the viri config, or a path containing `/`
    /// [default: every known garden; for new, the enclosing garden, then the config's default]
    #[arg(long, short, global = true, value_name = "NAME|PATH")]
    pub garden: Option<GardenArg>,

    #[command(subcommand)]
    pub command: Command,
}

/// How `--garden` names a garden. A value with a `/` in it, or `.` or
/// `..`, is a path; anything else is a name, so `./notes` reaches a
/// relative directory and `notes` never silently does.
#[derive(Clone, Debug, PartialEq)]
pub enum GardenArg {
    Path(Utf8PathBuf),
    Name(String),
}

impl FromStr for GardenArg {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if s.contains('/') || s == "." || s == ".." {
            GardenArg::Path(s.into())
        } else {
            GardenArg::Name(s.to_owned())
        })
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a node, optionally with parents
    New {
        /// Parent node id prefixes
        #[arg(value_name = "PREFIX")]
        parents: Vec<Prefix>,
    },
    /// Edit a node in $VISUAL or $EDITOR
    Edit {
        /// The node to edit
        prefix: Prefix,
    },
    /// List nodes: short ids, slugs, and tags
    Ls,
    /// Print a node in the edit buffer's format
    Show {
        /// The node to print
        prefix: Prefix,
    },
    #[command(name = "__internal", hide = true)]
    Internal {
        #[command(subcommand)]
        command: Internal,
    },
}

/// Commands for tooling rather than people.
#[derive(Subcommand)]
pub enum Internal {
    /// Print the agent guide for this build of viri
    Skill,
}

/// The guide `viri __internal skill` prints, kept in the binary so it
/// matches the commands this build has.
pub const SKILL_GUIDE: &str = include_str!("skill.md");

pub async fn run(cli: Cli) -> Result<(), miette::Report> {
    match cli.command {
        Command::Internal {
            command: Internal::Skill,
        } => print!("{SKILL_GUIDE}"),
        Command::Ls => ls(&reading(cli.garden).await?),
        Command::Show { prefix } => show(&reading(cli.garden).await?, &prefix).await?,
        Command::Edit { prefix } => edit(&mut reading(cli.garden).await?, &prefix).await?,
        Command::New { parents } => new(cli.garden, &parents).await?,
    }
    Ok(())
}

/// The gardens a read covers: the one `--garden` names, or every garden
/// viri knows.
async fn reading(explicit: Option<GardenArg>) -> Result<Gardens, miette::Report> {
    let config = Config::load()?;
    let roots = match explicit_root(&config, explicit)? {
        Some(root) => vec![root],
        None => known_roots(&config)?,
    };
    if roots.is_empty() {
        return Err(GardenNotFound.into());
    }
    open_all(roots).await
}

/// Lists every node. A slug that more than one garden uses is qualified
/// as `<garden>/<slug>`; elsewhere the garden goes unsaid.
fn ls(gardens: &Gardens) {
    let ids = gardens.ids();
    let mut holders: BTreeMap<&str, BTreeSet<&Utf8Path>> = BTreeMap::new();
    for garden in gardens.iter() {
        for node in garden.nodes() {
            let slug = node.slug.as_deref().unwrap_or("(no slug)");
            holders.entry(slug).or_default().insert(garden.root());
        }
    }

    for garden in gardens.iter() {
        for node in garden.nodes() {
            let short = id::shortest_prefix(&node.id, &ids);
            let slug = node.slug.as_deref().unwrap_or("(no slug)");
            let shared = holders.get(slug).is_some_and(|roots| roots.len() > 1);
            let slug = if shared {
                format!("{}/{slug}", garden.name())
            } else {
                slug.to_owned()
            };
            let tags = node
                .tags
                .iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" ");
            if tags.is_empty() {
                println!("{short}  {slug}");
            } else {
                println!("{short}  {slug}  {tags}");
            }
        }
    }
}

async fn show(gardens: &Gardens, prefix: &Prefix) -> Result<(), miette::Report> {
    let (garden, id) = gardens.resolve(prefix)?;
    let node = garden.load(&id).await?;
    print!(
        "{}",
        crate::buffer::render(&Buffer::new(node.meta, node.body))
    );
    Ok(())
}

/// Creates a node in the target garden, with an id no known garden uses
/// so its prefixes stay unique across all of them.
async fn new(explicit: Option<GardenArg>, parents: &[Prefix]) -> Result<(), miette::Report> {
    let config = Config::load()?;
    let preferred = match explicit_root(&config, explicit)? {
        Some(root) => Some(root),
        None => implied_root(&config)?,
    };
    let mut roots = known_roots(&config)?;
    if let Some(root) = &preferred
        && !roots.contains(root)
    {
        roots.push(root.clone());
    }
    if roots.is_empty() {
        return Err(GardenNotFound.into());
    }
    let mut gardens = open_all(roots).await?;
    let target = match preferred {
        Some(root) => root,
        None => choose_root(&gardens)?,
    };
    let id = NodeId::generate_unused(&gardens.ids());
    let garden = gardens.get_mut(&target).ok_or(GardenNotFound)?;

    let mut meta = NodeMeta::new(id);
    for prefix in parents {
        meta.parents.push(garden.resolve(prefix)?);
    }
    let start = Buffer::new(meta, String::new());
    let edited = editor::run(garden.root(), &garden.ids(), start, &id)?;
    garden
        .save(
            &Node {
                meta: edited.meta.clone(),
                body: edited.body,
            },
            "new",
            None,
        )
        .await?;
    println!(
        "{}  {}",
        gardens.shortest_prefix(&id),
        edited.meta.slug.as_deref().unwrap_or_default()
    );
    Ok(())
}

async fn edit(gardens: &mut Gardens, prefix: &Prefix) -> Result<(), miette::Report> {
    let (garden, id) = gardens.resolve_mut(prefix)?;
    let node = garden.load(&id).await?;
    let old_slug = node.meta.slug.clone();
    let start = Buffer::new(node.meta.clone(), node.body);
    let edited = editor::run(garden.root(), &garden.ids(), start, &id)?;
    garden
        .save(
            &Node {
                meta: edited.meta,
                body: edited.body,
            },
            "edit",
            old_slug.as_deref(),
        )
        .await?;
    Ok(())
}

/// The garden a new node goes in without `--garden`: the enclosing
/// garden, else the config's default.
fn implied_root(config: &Config) -> Result<Option<Utf8PathBuf>, miette::Report> {
    if let Some(root) = enclosing_garden()? {
        return Ok(Some(canonical(&root)?));
    }
    match &config.default {
        Some(name) => Ok(Some(canonical(&config.find(name)?)?)),
        None => Ok(None),
    }
}

/// The only garden, or the one the user picks when there are several.
/// Without a terminal to ask on, several is an error.
fn choose_root(gardens: &Gardens) -> Result<Utf8PathBuf, miette::Report> {
    let choices: Vec<(&str, &Utf8Path)> = gardens.iter().map(|g| (g.name(), g.root())).collect();
    if let [(_, root)] = choices.as_slice() {
        return Ok(root.to_path_buf());
    }
    if std::io::stdin().is_terminal() {
        // The tests have no terminal; prompt.rs covers the prompt itself.
        // cov-excl-start
        let index = prompt::choose_garden(
            &choices,
            &mut std::io::stdin().lock(),
            &mut std::io::stderr(),
        )?;
        return choices
            .get(index)
            .map(|(_, root)| root.to_path_buf())
            .ok_or_else(|| GardenNotFound.into());
    }
    // cov-excl-stop
    Err(WhichGarden {
        roots: choices
            .iter()
            .map(|(_, root)| root.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    }
    .into())
}

/// The garden `--garden` names, if it was passed.
fn explicit_root(
    config: &Config,
    explicit: Option<GardenArg>,
) -> Result<Option<Utf8PathBuf>, miette::Report> {
    match explicit {
        None => Ok(None),
        Some(GardenArg::Path(path)) if path.join("viri.kdl").is_file() => {
            Ok(Some(canonical(&path)?))
        }
        Some(GardenArg::Path(path)) => Err(NotAGarden { path }.into()),
        Some(GardenArg::Name(name)) => Ok(Some(canonical(&config.find(&name)?)?)),
    }
}

/// Every garden viri knows: the configured ones and the enclosing one,
/// each once.
fn known_roots(config: &Config) -> Result<Vec<Utf8PathBuf>, miette::Report> {
    let mut roots = Vec::new();
    for root in config.gardens.iter().cloned().chain(enclosing_garden()?) {
        let root = canonical(&root)?;
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    Ok(roots)
}

async fn open_all(roots: Vec<Utf8PathBuf>) -> Result<Gardens, miette::Report> {
    let mut gardens = Vec::new();
    for root in roots {
        gardens.push(Garden::open(&root).await?);
    }
    Ok(Gardens::new(gardens)?)
}

/// Canonical, so the same garden reached by two paths counts once.
fn canonical(root: &Utf8Path) -> Result<Utf8PathBuf, Unreachable> {
    root.canonicalize_utf8().map_err(|source| Unreachable {
        root: root.to_owned(),
        source,
    })
}

/// The nearest ancestor of the working directory holding a `viri.kdl`.
/// One that can't be named as UTF-8 is an error rather than `None`, so
/// viri never falls back to the default garden from inside another one.
fn enclosing_garden() -> Result<Option<Utf8PathBuf>, GardenNotFound> {
    let cwd = std::env::current_dir().map_err(|_| GardenNotFound)?;
    let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
    while let Some(current) = dir {
        if current.join("viri.kdl").is_file() {
            // cov-excl-macos-start
            return Utf8PathBuf::try_from(current.to_owned())
                .map(Some)
                .map_err(|_| GardenNotFound);
            // cov-excl-macos-stop
        }
        dir = current.parent();
    }
    Ok(None)
}

#[derive(Error, Diagnostic, Debug)]
#[error("no garden found: not inside a garden, and the viri config lists none")]
#[diagnostic(
    code(viri::cli::garden_not_found),
    help(
        "run viri inside a garden, pass --garden <path>, or list gardens in $XDG_CONFIG_HOME/viri/config.kdl"
    )
)]
pub struct GardenNotFound;

#[derive(Error, Diagnostic, Debug)]
#[error("{path} is not a garden: it has no viri.kdl")]
#[diagnostic(code(viri::cli::not_a_garden))]
pub struct NotAGarden {
    path: Utf8PathBuf,
}

#[derive(Error, Diagnostic, Debug)]
#[error("can't reach the garden at {root}")]
#[diagnostic(
    code(viri::cli::unreachable_garden),
    help("fix the path, or remove it from the viri config")
)]
pub struct Unreachable {
    root: Utf8PathBuf,
    #[source]
    source: std::io::Error,
}

#[derive(Error, Diagnostic, Debug)]
#[error("which garden should the new node go in? viri knows {roots}")]
#[diagnostic(
    code(viri::cli::which_garden),
    help(
        "pass --garden <name|path>, set `default` in the viri config, or run viri in a terminal to choose"
    )
)]
pub struct WhichGarden {
    roots: String,
}
