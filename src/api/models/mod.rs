/// Data models for the changeset historic API
pub mod changeset;
pub mod metadata;

// Re-export types for convenient access
pub use changeset::Changeset;
pub use metadata::{
    BoundingBox, CoverageMetadata, PartitionInfo, TipMetadata, UserStats,
};
