//! Standalone Native Rust Engine for Daily News & Content Discovery.
//!
//! Follows Clean Architecture & Domain-Driven Design (DDD) principles:
//! - `domain`: Core business entities, value objects, sanitizers, and hashing.
//! - `date`: Pure date/time parsing, formatting, and boundary calculation.
//! - `ports`: Inbound/outbound port traits (`FeedFetcherPort`, `StoragePort`).
//! - `service`: Application use cases (`DailyNewsService`).
//! - `scoring`: Deterministic 6-axis scoring engine.
//! - `storage`: Persistent storage adapter (`FileStorageAdapter`).
//! - `fetcher`: Live feed fetcher adapter (`LiveFeedFetcher`).
//! - `redis`: Redis queue and stream feed provider.
//! - `ui`: Responsive HTML dashboard presenter.

pub mod arctic;
pub mod date;
pub mod discovery;
pub mod discussion;
pub mod domain;
pub mod fetcher;
pub mod hn;
pub mod ports;
pub mod reddit;
pub mod redis;
pub mod scoring;
pub mod service;
pub mod storage;
pub mod ui;
pub mod url;

pub use arctic::*;
pub use discovery::*;
pub use discussion::*;
pub use domain::*;
pub use fetcher::*;
pub use hn::*;
pub use ports::*;
pub use reddit::*;
pub use redis::*;
pub use scoring::*;
pub use service::*;
pub use storage::*;
pub use ui::*;
