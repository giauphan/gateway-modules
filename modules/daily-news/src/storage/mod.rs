//! Storage adapter for persistence, consumed tracking, and distributed handoff leases.
//!
//! Enforces Clean Architecture layer separation and file size limits through submodules.

pub mod file;
pub mod in_memory;
pub mod lock;
pub mod merge;

pub use file::{resolve_state_dir, FileStorageAdapter};
pub use in_memory::InMemoryStorageAdapter;
pub use lock::ProcessFileLock;
pub use merge::merge_candidate_entry;

#[cfg(test)]
mod tests;
