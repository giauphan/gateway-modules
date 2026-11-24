//! Standalone Native Rust stdio-stream entrypoint for Proxy Guard.
//!
//! Frameworks & Drivers layer: Composition root and stdio-stream event loop.
//! Reads JSON requests from stdin, dispatches to interface adapter handlers,
//! and writes JSON responses to stdout.

use proxy_guard::domain::{ProxyRotationStrategy, UpstreamProxy};
use proxy_guard::handlers::handle_request;
use proxy_guard::ports::ProxyPoolManagementPort;
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::proxy_server::ProxyServer;
use proxy_guard::rpc::make_json_resp;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use serde_json::{json, Value};
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("proxy-guard {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Proxy Guard & Traffic Optimizer v{}",
            env!("CARGO_PKG_VERSION")
        );
        println!("Usage: proxy-guard [options]");
        println!("Options:");
        println!("  -V, --version           Print version information");
        println!("  -h, --help              Print help information");
        println!(
            "  --proxy-port <PORT>     Run background HTTP/CONNECT intercepting proxy on port"
        );
        println!("  --upstream <URL>        Forward allowed traffic to upstream paid proxy");
        return;
    }

    let base_dir = std::env::var("PROXY_GUARD_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            if cur.join("modules").join("proxy-guard").is_dir() {
                cur.join("modules").join("proxy-guard")
            } else {
                cur
            }
        });

    let storage = Arc::new(FileStorageAdapter::new(&base_dir));
    let cost_usd = std::env::var("PROXY_GUARD_COST_PER_GB")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(3.5);

    let proxy_pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));

    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        proxy_pool,
        cost_usd,
    ));

    // Check optional proxy server port and upstream proxies from args or env
    let mut proxy_port: Option<u16> = std::env::var("PROXY_GUARD_PROXY_PORT")
        .ok()
        .and_then(|p| p.parse().ok());

    let mut raw_upstream_inputs: Vec<String> = Vec::new();
    if let Ok(pool_env) = std::env::var("PROXY_GUARD_UPSTREAM_POOL") {
        raw_upstream_inputs.push(pool_env);
    }
    if let Ok(up_env) = std::env::var("PROXY_GUARD_UPSTREAM") {
        raw_upstream_inputs.push(up_env);
    }

    let mut configured_strategy = std::env::var("PROXY_GUARD_ROTATION_STRATEGY").ok();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--proxy-port" && i + 1 < args.len() {
            proxy_port = args[i + 1].parse().ok();
            i += 2;
        } else if (args[i] == "--upstream" || args[i] == "--upstream-pool") && i + 1 < args.len() {
            raw_upstream_inputs.push(args[i + 1].clone());
            i += 2;
        } else if (args[i] == "--strategy" || args[i] == "--rotation-strategy")
            && i + 1 < args.len()
        {
            configured_strategy = Some(args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }

    // Configure rotation strategy if specified
    if let Some(ref strat_str) = configured_strategy {
        let strat = match strat_str.to_ascii_lowercase().as_str() {
            "round_robin" => ProxyRotationStrategy::RoundRobin,
            "random" => ProxyRotationStrategy::Random,
            "sticky_session" | "sticky" => ProxyRotationStrategy::StickySession,
            "lowest_failures" | "health" => ProxyRotationStrategy::LowestFailures,
            _ => ProxyRotationStrategy::RoundRobin,
        };
        let _ = service.set_strategy(strat);
    }

    // Populate upstream proxies (split by newline, comma, semicolon)
    for raw in raw_upstream_inputs {
        for line in raw.split(['\n', ',', ';']) {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Ok(parsed) = UpstreamProxy::parse(trimmed, None, None, None) {
                    let _ = service.add_proxy(parsed);
                }
            }
        }
    }

    let default_upstream = service.list_proxies().first().map(|p| p.url.clone());

    if let Some(port) = proxy_port {
        if port > 0 {
            let server = ProxyServer::new(port, default_upstream, Arc::clone(&service));
            match server.start_background() {
                Ok((_handle, bound_port)) => {
                    eprintln!(
                        "[proxy-guard] Local intercepting proxy listening on 127.0.0.1:{bound_port}"
                    );
                }
                Err(e) => {
                    eprintln!("[proxy-guard] Warning: Failed to start proxy server: {e}");
                }
            }
        }
    }

    // stdio-stream event loop for Toolcase Gateway
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp =
                    make_json_resp(400, json!({ "error": format!("Invalid JSON: {e}") }));
                println!("{}", serde_json::to_string(&err_resp).unwrap_or_default());
                continue;
            }
        };

        let resp = handle_request(req, &service);
        println!("{}", serde_json::to_string(&resp).unwrap_or_default());
    }
}
