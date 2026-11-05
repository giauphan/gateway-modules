//! Research readiness, claim verification, and source provenance contracts.
//!
//! Enforces Clean Architecture domain rules: separates verified primary source
//! facts from community discussions, and guards video production readiness.

pub mod evaluator;
pub mod media_decoder;
pub mod types;
pub mod verification;
pub mod visual;

pub use evaluator::*;
pub use types::*;
pub use verification::*;
pub use visual::*;
