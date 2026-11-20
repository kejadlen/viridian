//! A garden: metadata in `viri.kdl` joined with bodies in a jj repo.

use std::collections::BTreeSet;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use miette::Diagnostic;
use thiserror::Error;

use crate::body::Bodies;
use crate::body::BodiesError;
use crate::id;
use crate::id::NodeId;
use crate::id::PrefixError;
use crate::meta::Meta;
use crate::meta::MetaError;
use crate::schema::NodeMeta;
use crate::schema::ValidationError;

/// A whole node: its metadata plus its body.
#[derive(Clone, Debug)]
pub struct Node {
    pub meta: NodeMeta,
    pub body: String,
}

/// An open garden: the `viri.kdl` metadata store and the jj body store.
pub struct Garden {
    root: Utf8PathBuf,
    meta: Meta,
    bodies: Bodies,
}

impl Garden {
    /// Opens the garden rooted at `root`, which must contain `viri.kdl` and
    /// a jj workspace.
    pub async fn open(root: &Utf8Path) -> Result<Garden, GardenError> {
        let meta = Meta::open(&root.join("viri.kdl"))?;
        let bodies = Bodies::open(root).await?;
        Ok(Garden {
            root: root.to_owned(),
            meta,
            bodies,
        })
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    pub fn ids(&self) -> BTreeSet<NodeId> {
        self.meta.ids()
    }

    /// The shortest prefix of `id` unique within the garden.
    pub fn shortest_prefix(&self, id: &NodeId) -> String {
        id::shortest_prefix(id, &self.ids())
    }

    /// Resolves a prefix to the unique node it names.
    pub fn resolve(&self, prefix: &str) -> Result<NodeId, PrefixError> {
        id::resolve_prefix(prefix, &self.ids())
    }

    /// A fresh id the garden doesn't use yet.
    pub fn generate_id(&self) -> NodeId {
        loop {
            let id = NodeId::generate();
            if !self.ids().contains(&id) {
                return id;
            }
        }
    }

    /// Loads a node: metadata plus body. The body is empty when the body file
    /// doesn't exist yet.
    pub async fn load(&self, id: &NodeId) -> Result<Node, GardenError> {
        let meta = self.meta.get(id).ok_or(GardenError::UnknownNode(*id))?;
        let body = self.bodies.read_body(id)?;
        Ok(Node {
            meta: meta.clone(),
            body,
        })
    }

    /// Saves a node: body commits first, metadata write second, so a crash
    /// between the two leaves a body no metadata lists.
    ///
    /// `old_slug` is the slug the metadata had before this save; a changed
    /// slug renames the body file in a commit of its own.
    pub async fn save(
        &mut self,
        node: &Node,
        verb: &str,
        old_slug: Option<&str>,
    ) -> Result<(), GardenError> {
        let errors = node.meta.validate(&self.ids());
        if !errors.is_empty() {
            return Err(GardenError::Invalid(errors));
        }
        let slug = node.meta.slug.as_deref().unwrap_or_default();
        self.bodies
            .write_body(&node.meta.id, old_slug, slug, &node.body, verb)
            .await?;
        self.meta.upsert(node.meta.clone());
        self.meta.write()?;
        Ok(())
    }

    /// Every node's metadata, in file order, which is creation order.
    pub fn nodes(&self) -> impl Iterator<Item = &NodeMeta> {
        self.meta.nodes()
    }
}

#[derive(Error, Diagnostic, Debug)]
pub enum GardenError {
    #[error(transparent)]
    #[diagnostic(code(viri_core::garden::meta))]
    Meta(#[from] MetaError),

    #[error(transparent)]
    #[diagnostic(code(viri_core::garden::body))]
    Body(#[from] BodiesError),

    #[error("no node {0}")]
    #[diagnostic(code(viri_core::garden::unknown_node))]
    UnknownNode(NodeId),

    #[error("validation failed")]
    #[diagnostic(code(viri_core::garden::invalid))]
    Invalid(#[related] Vec<ValidationError>),
}
