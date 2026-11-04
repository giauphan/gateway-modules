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
pub mod content_quality;
pub mod date;
pub mod discovery;
pub mod discussion;
pub mod domain;
pub mod enrichment;
pub mod fetcher;
pub mod handlers;
pub mod hn;
pub mod mcp;
pub mod mcp_schema;
pub mod ports;
pub mod reddit;
pub mod redis;
pub mod research_readiness;
pub mod rpc;
pub mod scoring;
pub mod service;
pub mod source_capture;
pub mod storage;
pub mod ui;
pub mod url;

pub use arctic::*;
pub use discovery::*;
pub use discussion::*;
pub use domain::*;
pub use enrichment::*;
pub use fetcher::*;
pub use handlers::*;
pub use hn::*;
pub use mcp::*;
pub use ports::*;
pub use reddit::*;
pub use redis::*;
pub use rpc::*;
pub use scoring::*;
pub use service::*;
pub use source_capture::*;
pub use storage::*;
pub use ui::*;
