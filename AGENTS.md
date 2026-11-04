# Agent Rules for Toolcase Gateway Modules

This document establishes the architecture, language, and quality standards for all feature modules and extensions in this repository.

---

## 1. Top-Priority Implementation: Native Rust

- **Rust as Top Priority**: All feature modules, extensions, and personal modules MUST be authored in Rust as the primary implementation language.
- **Zero-Python Runtime Target**: Avoid Python or interpreted runtimes for production modules. Modules must compile to a native, self-contained Rust binary to eliminate startup latency, GIL contention, packaging fragility, and host runtime requirements.
- **Self-Contained Crate Layout**:
  Every module under `modules/<module-id>/` must be a self-contained Rust project:
  ```text
  modules/<module-id>/
  ├── Cargo.toml          # Module crate definition and dependencies
  ├── module.json         # Declarative gateway manifest
  ├── run.sh              # Executable launcher (chmod +x)
  ├── <module-id>         # Precompiled release binary
  ├── src/                # Clean Architecture & DDD source files
  └── tests/              # Comprehensive integration test suite
  ```
- **Precompiled Binary Execution**:
  The `run.sh` entrypoint must execute the precompiled binary directly for sub-millisecond execution times:
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
  - `domain.rs`: Pure domain entities, value objects, and deterministic business rules. Zero transport or framework dependencies.
  - `ports.rs`: Inbound and outbound trait definitions (`FeedProvider`, `StorageAdapter`, etc.).
  - `service.rs`: Core domain orchestrator / application service implementing the business rules.
  - `fetcher.rs` / `redis.rs` / `storage.rs`: Infrastructure adapters implementing the port traits.
  - `main.rs`: Stdio-stream / JSON-RPC framing adapter and entrypoint.
- **Strict File-Size Ceilings**:
  - Target: **under 450 lines** per source file.
  - Hard ceiling: **800 lines** maximum. Files exceeding 800 lines must be split into cohesive submodules.
- **Boundary Validation**:
  - Validate and sanitize all external inputs, URLs, and JSON payloads at system entry boundaries. Fail closed on unexpected or malformed inputs.
  - SSRF protection: Block private IP ranges (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`) and strip tracking parameters (`utm_*`, `fbclid`).

---

## 3. Testing & Verification

- Every module must include automated tests under `tests/` covering:
  - Normalization and deterministic hashing (`title_hash`, `content_hash`).
  - Filtering and validation logic (dates, topics, limits).
  - Out-of-process protocol framing (`stdio-stream` JSON-RPC).
  - Failure modes and offline resilience.
- Run `cargo test` and ensure all tests pass with zero warnings before committing changes.

---

## 4. Version Management & Single Source of Truth

- **Mandatory Version Upgrade on Code Changes**:
  - Every functional modification, bug fix, feature addition, or refactor MUST bump the module's semantic version (`MAJOR.MINOR.PATCH`).
  - `PATCH` (`1.5.0` -> `1.5.1`): Bug fixes, hotfixes, and internal logic corrections.
  - `MINOR` (`1.5.0` -> `1.6.0`): New capabilities, non-breaking endpoints, or configuration fields.
  - `MAJOR` (`1.0.0` -> `2.0.0`): Breaking changes to payload schemas, protocol contracts, or namespace mounting.
- **Single Source of Truth (`Cargo.toml`)**:
  - The module's `Cargo.toml` (`version = "..."`) is the authoritative single source of truth for the version.
  - Rust source files MUST NEVER hardcode version strings. Always reference the crate version at compile time via `env!("CARGO_PKG_VERSION")`.
  - Non-Rust manifests (`module.json`), documentation (`docs/`), and precompiled binaries are synchronized from `Cargo.toml` using the automated script:
    ```bash
    # Option A: Bump version in Cargo.toml and synchronize all files
    ./scripts/sync-version.sh <new_version>

    # Option B: Synchronize all files after manually editing Cargo.toml
    ./scripts/sync-version.sh
    ```

---

## 5. Module Model Context Protocol (MCP) Support for AI Agents

Toolcase Gateway acts as an extensible MCP gateway, allowing modules to contribute their domain capabilities directly to AI agents in standard MCP format:

- **Native MCP Tool & Resource Exposure**:
  - Platform modules (both personal out-of-process modules and built-in providers) can expose custom tools and resources to connected AI agents.
  - Agents connecting via any supported transport (stdio `--mcp`, Streamable HTTP `POST /mcp`, or SSE `GET /mcp/sse`) discover and call module tools identically.
- **Manifest Tool Declarations (`tools` array)**:
  - Modules declare custom tools in `module.json` using the `tools` array with standard JSON Schema `inputSchema`:
    ```json
    {
      "name": "my-module",
      "tools": [
        {
          "name": "query_items",
          "description": "Search and retrieve items from the module catalog",
          "inputSchema": {
            "type": "object",
            "properties": {
              "query": { "type": "string", "description": "Search keyword" }
            },
            "required": ["query"]
          }
        }
      ]
    }
    ```
- **Capability-Driven Tool Synthesis**:
  - If a module omits the explicit `tools` array, the gateway automatically synthesizes default MCP tools based on its declared `capabilities`: `chat_completions`/`chat` → `<id>__chat`, `models` → `<id>__list_models`, `decisions` → `<id>__evaluate`, `image` → `<id>__generate_image`.
- **Namespacing & Isolation**:
  - Module tools are aggregated and exposed under namespaced names: `<module_id>__<tool_name>` (e.g., `daily-news__fetch_latest_candidates`).
  - Built-in administrative tools retain their canonical names (`gateway_list_modules`, `gateway_get_module_info`, etc.), preventing collisions.
- **Execution Lifecycle & Timeouts**:
  - Module tool executions over stdio processes have a strict 30-second bounded timeout to protect synchronous gateway worker threads.
  - Disabled or uninstalled modules fail closed: their tools are immediately excluded from `tools/list` and `resources/list`, and any direct call is rejected with standard MCP errors.
- **Clean Architecture MCP Adapter Handling**:
  - Modules must handle incoming tool executions via `POST /tools/call` over stdio-stream framing.
  - Interface adapters deserialize `{ "name": "<action>", "arguments": { ... } }`, validate inputs strictly, delegate to domain use cases, and serialize the result into standard MCP content responses:
    ```json
    {
      "content": [
        {
          "type": "text",
          "text": "..."
        }
      ]
    }
    ```


