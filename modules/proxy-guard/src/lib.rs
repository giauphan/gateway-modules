//! Proxy Guard: High-Performance Proxy Traffic Optimizer & Bandwidth Saver.
//!
//! Follows Clean Architecture and Domain-Driven Design (DDD) principles:
//! - `domain`: Pure business rules, entities, and value objects.
//! - `ports`: Inbound (driving) and Outbound (driven) port traits.
//! - `service`: Application orchestrator use-cases.
//! - `rules_engine`: Domain pattern matching & builtin catalog.
//! - `mock_factory`: Synthetic mock payload generation.
//! - `script_generator`: Client automation interceptor code generation.
//! - `storage`: File & in-memory persistence adapter.
//! - `proxy_server`: Native multithreaded intercepting proxy server.
//! - `handlers`: Interface adapters for HTTP/JSON-RPC routes.
//! - `rpc`: JSON framing helpers.
//! - `mcp`: Model Context Protocol tool execution.

pub mod domain;
pub mod handlers;
pub mod mcp;
pub mod mock_factory;
pub mod ports;
pub mod proxy_server;
pub mod request_optimizer;
pub mod rpc;
pub mod rules_engine;
pub mod script_generator;
pub mod service;
pub mod storage;

pub use domain::*;
pub use ports::*;
pub use service::*;
