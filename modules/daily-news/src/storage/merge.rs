//! Storage-side candidate cache rules.
//!
//! Candidate revision merge policy is a pure domain concern and lives in
//! `crate::domain::candidate_merge`; it is re-exported here so storage adapters
//! and callers keep a stable facade.

pub use crate::domain::merge_candidate_entry;

pub fn trim_candidate_cache(guard: &mut Vec<crate::domain::NormalizedCandidate>, max_size: usize) {
    if guard.len() > max_size {
        let excess = guard.len() - max_size;
        guard.drain(0..excess);
    }
}