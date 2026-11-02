# Toolcase Gateway Modules Storage Repository

Welcome to the **Toolcase Gateway Modules** repository.

This repository serves as the centralized storage and distribution catalog for modular out-of-process provider extensions, adapters, and tools for [Toolcase Gateway](https://github.com/giauphan/toolcase-gateway).

---

## 1. Repository Structure

All standalone feature modules reside under the [`modules/`](modules/) directory. Each module is completely self-contained with its own manifest, protocol adapter, dependencies, and test suite:

```text
gateway-modules/
├── README.md                 # Repository overview and module index
├── .gitignore
└── modules/                  # Feature modules directory
    ├── daily-news/           # Daily News & Content Discovery Service
    │   ├── module.json       # Declarative manifest (ID, namespace, capabilities)
    │   ├── run.sh            # Standard executable entrypoint launcher
    │   ├── adapter.py        # stdio-stream protocol adapter
    │   ├── config.example.json
    │   ├── README.md         # Module documentation and API reference
    │   ├── daily_news/       # Core Python domain and application package
    │   └── tests/            # Module unit tests
    │
    └── <future-module>/      # Future feature modules can be added here
        ├── module.json
        ├── run.sh
        ├── adapter.py
        └── README.md
```

---

## 2. Available Feature Modules

| Module ID | Name | Namespace | Capabilities | Path |
| :--- | :--- | :--- | :--- | :--- |
| **`daily-news`** | **Daily News & Content Discovery Service** | `/custom/daily-news` | `news`, `content_discovery`, `chat_completions`, `models` | [`modules/daily-news`](modules/daily-news) |

### Module Highlights: `daily-news`
- **Multi-Source Ingestion**: Ingests breaking updates from Hacker News, GitHub Trending/Releases, Product Hunt, Reddit, and ArXiv.
- **Official Specification Feeds**: Ingests official documentation and specs from `modelcontextprotocol.io`, `docs.anthropic.com`, `docs.github.com`, and `git-scm.com`.
- **SSRF Safety & URL Canonicalization**: Bounded HTTPS checking, loopback/private IP blocking, and tracking parameter stripping (`utm_*`, `fbclid`).
- **Deterministic Deduplication**: Computes SHA-256 `title_hash` and `content_hash` to eliminate duplicates.
- **Multi-Axis Scoring Engine**: Ranks content along six axes (freshness, publisher authority, fact density, visual potential, audience interest, feasibility).
- **Tiered Discovery**: Priority cascading (Tier 1 Curated Specs $\to$ Tier 2 Automated Feeds $\to$ Tier 3 Fail-Closed).

---

## 3. How to Add a New Feature Module

To add a new module to this repository:

1. Create a dedicated folder under `modules/<your-module-id>/`:
   ```bash
   mkdir -p modules/my-feature-module
   cd modules/my-feature-module
   ```
2. Define the declarative manifest `module.json`:
   ```json
   {
     "id": "my-feature-module",
     "name": "My Feature Module",
     "version": "1.0.0",
     "namespace": "/custom/my-feature",
     "entrypoint": "run.sh",
     "protocol": "stdio-stream",
     "capabilities": ["models", "chat_completions"]
   }
   ```
3. Provide the executable launcher `run.sh` (`chmod +x run.sh`):
   ```bash
   #!/bin/sh
   SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
   exec python3 "$SCRIPT_DIR/adapter.py" "$@"
   ```
4. Implement the `stdio-stream` JSON-RPC handler in `adapter.py` reading requests from `stdin` and returning responses on `stdout`.
5. Add unit tests under `tests/` and document your module in `README.md`.
6. Commit and push your changes to this repository.

---

## 4. Installation in Toolcase Gateway

Modules in this repository can be installed into Toolcase Gateway via the web dashboard at `http://127.0.0.1:28130/dashboard` or via the gateway REST API:

```bash
curl -X POST http://127.0.0.1:28130/api/v1/dashboard/modules/install \
  -H "Content-Type: application/json" \
  -d '{
    "source": "https://github.com/giauphan/gateway-modules.git",
    "id": "daily-news"
  }'
```
