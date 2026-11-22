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

pub mod body;
pub mod garden;
pub mod gardens;
pub mod id;
pub mod meta;
pub mod schema;

pub use garden::Garden;
pub use garden::Node;
pub use gardens::Gardens;
pub use id::NodeId;
pub use schema::NodeMeta;
pub use schema::ValidationError;
