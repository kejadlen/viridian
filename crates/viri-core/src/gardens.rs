//! Several open gardens, addressed as one: a node id names the same node
//! whichever garden holds it, so prefixes resolve across all of them.

use std::collections::BTreeSet;

use camino::Utf8Path;
use miette::Diagnostic;
use thiserror::Error;

use crate::garden::Garden;
use crate::id;
use crate::id::NodeId;
use crate::id::Prefix;
use crate::id::PrefixError;

pub struct Gardens {
    gardens: Vec<Garden>,
}

impl Gardens {
    /// Joins gardens whose node ids don't overlap; an id in two gardens
    /// would name two nodes.
    pub fn new(gardens: Vec<Garden>) -> Result<Gardens, GardensError> {
        let mut seen = BTreeSet::new();
        for garden in &gardens {
            for id in garden.ids() {
                if !seen.insert(id) {
                    return Err(GardensError::SharedId { id });
                }
            }
        }
        Ok(Gardens { gardens })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Garden> {
        self.gardens.iter()
    }

    pub fn get_mut(&mut self, root: &Utf8Path) -> Option<&mut Garden> {
        self.gardens.iter_mut().find(|garden| garden.root() == root)
    }

    /// Every node id in every garden.
    pub fn ids(&self) -> BTreeSet<NodeId> {
        self.gardens.iter().flat_map(Garden::ids).collect()
    }

    /// The shortest prefix of `id` unique across all the gardens.
    pub fn shortest_prefix(&self, id: &NodeId) -> Prefix {
        id::shortest_prefix(id, &self.ids())
    }

    /// Resolves a prefix to the unique node it names and the garden
    /// holding it.
    pub fn resolve(&self, prefix: &Prefix) -> Result<(&Garden, NodeId), PrefixError> {
        let id = id::resolve_prefix(prefix, &self.ids())?;
        let garden = self.gardens.iter().find(|g| g.ids().contains(&id));
        Ok((
            garden.ok_or_else(|| PrefixError::NotFound(prefix.clone()))?,
            id,
        ))
    }

    pub fn resolve_mut(&mut self, prefix: &Prefix) -> Result<(&mut Garden, NodeId), PrefixError> {
        let id = id::resolve_prefix(prefix, &self.ids())?;
        let garden = self.gardens.iter_mut().find(|g| g.ids().contains(&id));
        Ok((
            garden.ok_or_else(|| PrefixError::NotFound(prefix.clone()))?,
            id,
        ))
    }
}

#[derive(Error, Diagnostic, Debug)]
pub enum GardensError {
    #[error("node {id} is in two gardens")]
    #[diagnostic(
        code(viri_core::gardens::shared_id),
        help("an id names one node; remove it from one garden's viri.kdl")
    )]
    SharedId { id: NodeId },
}
