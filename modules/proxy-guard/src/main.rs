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

struct CliConfig {
    proxy_port: Option<u16>,
    raw_upstream_inputs: Vec<String>,
    configured_strategy: Option<String>,
}

fn parse_cli_and_env(args: &[String]) -> Result<Option<CliConfig>, ()> {
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("proxy-guard {}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
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
        return Ok(None);
    }

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

    Ok(Some(CliConfig {
        proxy_port,
        raw_upstream_inputs,
        configured_strategy,
    }))
}

fn resolve_base_dir() -> PathBuf {
    std::env::var("PROXY_GUARD_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            if cur.join("modules").join("proxy-guard").is_dir() {
                cur.join("modules").join("proxy-guard")
            } else {
                cur
            }
        })
}

fn init_proxies(
    service: &ProxyGuardService,
    raw_inputs: Vec<String>,
    strategy: Option<String>,
    proxy_port: Option<u16>,
    base_dir: &std::path::Path,
) -> Option<u16> {
    let mut effective_strategy = strategy;
    let mut effective_port = proxy_port;

    let config_candidates = [
        base_dir.join("config.json"),
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("config.json"),
    ];

    for cfg_path in &config_candidates {
        if cfg_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(cfg_path) {
                if let Ok(val) = serde_json::from_str::<Value>(&content) {
                    if effective_port.is_none() {
                        if let Some(port_num) =
                            val.get("proxy_listener_port").and_then(|v| v.as_u64())
                        {
                            if port_num > 0 && port_num <= 65535 {
                                effective_port = Some(port_num as u16);
                            }
                        }
                    }
                    if effective_strategy.is_none() {
                        if let Some(strat_str) =
                            val.get("rotation_strategy").and_then(|v| v.as_str())
                        {
                            let trimmed = strat_str.trim();
                            if !trimmed.is_empty() {
                                effective_strategy = Some(trimmed.to_string());
                            }
                        }
                    }
                    if service.list_proxies().is_empty() {
                        if let Some(pool_raw) =
                            val.get("upstream_proxy_pool").and_then(|v| v.as_str())
                        {
                            for line in pool_raw.split(['\n', ',', ';']) {
                                let trimmed = line.trim();
                                if !trimmed.is_empty() {
                                    if let Ok(parsed) =
                                        UpstreamProxy::parse(trimmed, None, None, None)
                                    {
                                        let _ = service.add_proxy(parsed);
                                    }
                                }
                            }
                        }
                        if let Some(single_url) =
                            val.get("upstream_proxy_url").and_then(|v| v.as_str())
                        {
                            let trimmed = single_url.trim();
                            if !trimmed.is_empty() {
                                if let Ok(parsed) = UpstreamProxy::parse(trimmed, None, None, None)
                                {
                                    let _ = service.add_proxy(parsed);
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    if let Some(strat_str) = effective_strategy {
        let strat = match strat_str.to_ascii_lowercase().as_str() {
            "round_robin" | "roundrobin" => ProxyRotationStrategy::RoundRobin,
            "random" => ProxyRotationStrategy::Random,
            "sticky_session" | "sticky" => ProxyRotationStrategy::StickySession,
            "lowest_failures" | "health" | "healthy" => ProxyRotationStrategy::LowestFailures,
            _ => ProxyRotationStrategy::RoundRobin,
        };
        let _ = service.set_strategy(strat);
    }

    for raw in raw_inputs {
        for line in raw.split(['\n', ',', ';']) {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Ok(parsed) = UpstreamProxy::parse(trimmed, None, None, None) {
                    let _ = service.add_proxy(parsed);
                }
            }
        }
    }

    effective_port
}

fn start_optional_proxy_server(proxy_port: Option<u16>, service: Arc<ProxyGuardService>) {
    if let Some(port) = proxy_port {
        if port > 0 {
            let default_upstream = service.list_proxies().first().map(|p| p.url.clone());
            let server = ProxyServer::new(port, default_upstream, service);
            match server.start_background() {
                Ok((_handle, bound_port)) => {
                    eprintln!(
                        "[proxy-guard] Intercepting traffic optimizer proxy listening on 0.0.0.0:{bound_port}"
                    );
                }
                Err(e) => {
                    eprintln!("[proxy-guard] Warning: Failed to start proxy server: {e}");
                }
            }
        }
    }
}

fn run_stdio_loop(service: &ProxyGuardService) {
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

        let resp = handle_request(req, service);
        println!("{}", serde_json::to_string(&resp).unwrap_or_default());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let config = match parse_cli_and_env(&args) {
        Ok(Some(cfg)) => cfg,
        _ => return,
    };

    let base_dir = resolve_base_dir();
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

    let effective_port = init_proxies(
        &service,
        config.raw_upstream_inputs,
        config.configured_strategy,
        config.proxy_port,
        &base_dir,
    );
    start_optional_proxy_server(effective_port, Arc::clone(&service));
    run_stdio_loop(&service);
}
