//! Standalone Native Rust Engine for Daily News & Content Discovery.
//!
//! Follows Clean Architecture & Domain-Driven Design (DDD) principles:
//! - `domain`: Core business entities, value objects, sanitizers, and hashing.
//! - `ports`: Inbound/outbound port traits (`FeedFetcherPort`, `StoragePort`).
//! - `service`: Application use cases (`DailyNewsService`).
//! - `scoring`: Deterministic 6-axis scoring engine.
//! - `storage`: Persistent storage adapter (`FileStorageAdapter`).
//! - `fetcher`: Live feed fetcher adapter (`LiveFeedFetcher`).
//! - `ui`: Responsive HTML dashboard presenter.

pub mod domain;
pub mod fetcher;
pub mod ports;
pub mod scoring;
pub mod service;
pub mod storage;
pub mod ui;

pub use domain::*;
pub use fetcher::*;
pub use ports::*;
pub use scoring::*;
pub use service::*;
pub use storage::*;
pub use ui::*;
