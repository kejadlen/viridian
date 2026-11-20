//! The `viri` command line: new, edit, ls, and show.

use std::path::PathBuf;

use camino::Utf8PathBuf;
use clap::Parser;
use clap::Subcommand;
use miette::Diagnostic;
use thiserror::Error;
use viri_core::Garden;
use viri_core::Node;
use viri_core::schema::NodeMeta;

use crate::buffer::Buffer;
use crate::editor;

#[derive(Parser)]
#[command(name = "viri", about = "Tend a viridian garden")]
pub struct Cli {
    /// The garden's root directory [default: the nearest ancestor holding a viri.kdl]
    #[arg(long, short, global = true)]
    pub garden: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a node, optionally with parents
    New {
        /// Parent node id prefixes
        #[arg(value_name = "PREFIX")]
        parents: Vec<String>,
    },
    /// Edit a node in $VISUAL or $EDITOR
    Edit {
        /// The node to edit
        prefix: String,
    },
    /// List nodes: short ids, slugs, and tags
    Ls,
    /// Print a node in the edit buffer's format
    Show {
        /// The node to print
        prefix: String,
    },
}

pub async fn run(cli: Cli) -> Result<(), miette::Report> {
    let root = find_garden(cli.garden)?;
    let mut garden = Garden::open(&root).await?;
    match cli.command {
        Command::Ls => ls(&garden),
        Command::Show { prefix } => show(&garden, &prefix).await?,
        Command::New { parents } => new(&mut garden, &parents).await?,
        Command::Edit { prefix } => edit(&mut garden, &prefix).await?,
    }
    Ok(())
}

fn ls(garden: &Garden) {
    for node in garden.nodes() {
        let short = garden.shortest_prefix(&node.id);
        let slug = node.slug.as_deref().unwrap_or("(no slug)");
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

async fn show(garden: &Garden, prefix: &str) -> Result<(), miette::Report> {
    let id = garden.resolve(prefix)?;
    let node = garden.load(&id).await?;
    print!(
        "{}",
        crate::buffer::render(&Buffer::new(node.meta, node.body), &[])
    );
    Ok(())
}

async fn new(garden: &mut Garden, parents: &[String]) -> Result<(), miette::Report> {
    let id = garden.generate_id();
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
        garden.shortest_prefix(&id),
        edited.meta.slug.as_deref().unwrap_or_default()
    );
    Ok(())
}

async fn edit(garden: &mut Garden, prefix: &str) -> Result<(), miette::Report> {
    let id = garden.resolve(prefix)?;
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

fn find_garden(explicit: Option<PathBuf>) -> Result<Utf8PathBuf, GardenNotFound> {
    if let Some(path) = explicit {
        let path = Utf8PathBuf::try_from(path).map_err(|_| GardenNotFound)?;
        if path.join("viri.kdl").is_file() {
            return Ok(path);
        }
        return Err(GardenNotFound);
    }
    let cwd = std::env::current_dir().map_err(|_| GardenNotFound)?;
    let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
    while let Some(current) = dir {
        if current.join("viri.kdl").is_file() {
            return Utf8PathBuf::try_from(current.to_owned()).map_err(|_| GardenNotFound);
        }
        dir = current.parent();
    }
    Err(GardenNotFound)
}

#[derive(Error, Diagnostic, Debug)]
#[error("no garden found: expected a viri.kdl in this directory or an ancestor")]
#[diagnostic(
    code(viri::cli::garden_not_found),
    help("run viri inside a garden, or pass --garden <path>")
)]
pub struct GardenNotFound;
