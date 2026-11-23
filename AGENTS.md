# Agent Rules for Toolcase Gateway Modules

Architecture, language, and quality standards for every module in this repository.

---

## 1. Top-Priority Implementation: Native Rust

- **Rust as Top Priority**: All feature modules, extensions, and personal modules MUST be authored in Rust.
- **Zero-Python Runtime Target**: No Python or interpreted runtime for production modules. Modules compile to a native, self-contained Rust binary — no startup latency, GIL contention, packaging fragility, or host runtime requirement.
- **Self-Contained Crate Layout**: every module under `modules/<module-id>/` is a self-contained Rust project:
  ```text
  modules/<module-id>/
  ├── Cargo.toml          # crate definition and dependencies
  ├── module.json         # declarative gateway manifest
  ├── run.sh              # executable launcher (chmod +x)
  ├── <module-id>         # precompiled release binary
  ├── src/                # Clean Architecture & DDD sources
  └── tests/              # integration test suite
  ```
- **Precompiled Binary Execution**: `run.sh` execs the precompiled binary for sub-millisecond startup, falling back to a release build:
  ```bash
  #!/bin/sh
  SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
  [ -f "$SCRIPT_DIR/<module-id>" ] && chmod +x "$SCRIPT_DIR/<module-id>" 2>/dev/null
  if [ -x "$SCRIPT_DIR/<module-id>" ]; then
    exec "$SCRIPT_DIR/<module-id>" "$@"
  fi
  exec cargo run --release --manifest-path "$SCRIPT_DIR/Cargo.toml" -- "$@"
  ```

---

## 2. Clean Architecture & Domain-Driven Design (DDD)

- **Layer Separation**:
  - `domain.rs`: pure domain entities, value objects, deterministic business rules. Zero transport/framework dependencies.
  - `ports.rs`: inbound and outbound trait definitions (`FeedProvider`, `StorageAdapter`, ...).
  - `service.rs`: domain orchestrator / application service.
  - `fetcher.rs` / `redis.rs` / `storage.rs`: infrastructure adapters implementing the port traits.
  - `main.rs`: stdio-stream / JSON-RPC framing adapter and entrypoint.
- **Strict File-Size Ceilings**: target **under 450 lines** per source file; hard ceiling **800 lines**. Larger files must be split into cohesive submodules.
- **Boundary Validation**: validate and sanitize all external inputs, URLs, and JSON payloads at system entry boundaries. Fail closed on unexpected or malformed input.
- **SSRF protection**: block private ranges (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`) and strip tracking parameters (`utm_*`, `fbclid`).

---

## 3. Testing & Verification

- Every module includes automated tests under `tests/` covering:
  - normalization and deterministic hashing (`title_hash`, `content_hash`);
  - filtering and validation logic (dates, topics, limits);
  - out-of-process protocol framing (`stdio-stream` JSON-RPC);
  - failure modes and offline resilience.
- `cargo test` must pass with zero warnings before committing.

---

## 4. Version Management & Single Source of Truth

- **Mandatory version bump on every functional change**:
  - `PATCH` (`1.5.0` → `1.5.1`): bug fixes, hotfixes, internal logic corrections.
  - `MINOR` (`1.5.0` → `1.6.0`): new capabilities, non-breaking endpoints, configuration fields.
  - `MAJOR` (`1.0.0` → `2.0.0`): breaking payload schemas, protocol contracts, namespace mounting.
- **Single source of truth (`Cargo.toml`)**: the module's `Cargo.toml` `version` is authoritative. Rust source must never hardcode a version string — use `env!("CARGO_PKG_VERSION")`.
- `module.json`, docs, and precompiled binaries are synchronized from `Cargo.toml` by `./scripts/sync-version.sh <new_version>` (or with no argument to re-sync after editing `Cargo.toml`).

---

## 5. Module Model Context Protocol (MCP) Support for AI Agents

Toolcase Gateway is an extensible MCP gateway: modules contribute domain capabilities directly to AI agents.

- **Exposure & transports**: modules expose tools and resources over stdio (`--mcp`), Streamable HTTP (`POST /mcp`), or SSE (`GET /mcp/sse`); agents discover and call them identically on every transport.
- **Manifest tool declarations**: declare tools in `module.json` with a standard JSON Schema `inputSchema`:
  ```json
  {"name":"my-module","tools":[{"name":"query_items","description":"Search the module catalog","inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"Search keyword"}},"required":["query"]}}]}
  ```
- **Capability-driven synthesis**: without an explicit `tools` array, the gateway synthesizes tools from `capabilities`: `chat_completions`/`chat` → `<id>__chat`, `models` → `<id>__list_models`, `decisions` → `<id>__evaluate`, `image` → `<id>__generate_image`.
- **Namespacing & isolation**: module tools are exposed as `<module_id>__<tool_name>` (e.g. `daily-news__fetch_latest_candidates`); built-in admin tools keep canonical names (`gateway_list_modules`, `gateway_get_module_info`, ...).
- **Lifecycle & timeouts**: stdio tool execution has a strict 30-second timeout. Disabled or uninstalled modules fail closed — excluded from `tools/list` and `resources/list`, direct calls rejected.
- **Adapter handling**: modules handle tool execution via `POST /tools/call` over stdio-stream framing. Adapters deserialize `{"name":"<action>","arguments":{...}}`, validate strictly, delegate to domain use cases, and return `{"content":[{"type":"text","text":"..."}]}`.

---

## 6. Rust Implementation Standards

Non-negotiable rules — full decision tables and worked examples in [`docs/RUST_SAFETY_RULES.md`](docs/RUST_SAFETY_RULES.md):

- No `unwrap()` / `expect()` in production code: use `?`, `unwrap_or*`, `match` / `if let`. `expect()` only at startup or in tests.
- No `.clone()` to silence the borrow checker: borrow (`&str`, `&[T]`), use lifetimes for views, `Cow` for read-mostly data, `Arc` for `'static` sharing.
- No `&String` / `&Vec<T>` parameters; no index loops; no `+` concatenation in loops; no `let _ = fallible_fn()`.
- No `unsafe` to bypass borrow/lifetime errors — FFI only, with a `// SAFETY:` comment.
- `thiserror` in domain/application; `anyhow` + `.context(...)` in adapters/bin.
- Never hold a synchronous lock guard across `.await`; preallocate with `with_capacity` when the size is known.
- Gate: `cargo clippy --all-targets --all-features -- -D warnings`.