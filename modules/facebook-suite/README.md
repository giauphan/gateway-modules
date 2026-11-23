# Meta Business Suite / Facebook Creator Studio Adapter

The **Meta Business Suite / Facebook Creator Studio Adapter** is an out-of-process personal module addon for **Toolcase Gateway**. It bridges the gateway with Meta Business Suite, Facebook Creator Studio, and Meta's resumable video upload (`rupload`) services.

---

## 1. Features & Capabilities

- **1-Click HAR Import**: Drag-and-drop or upload `download-facebook.har` to auto-extract authenticated session credentials (`__user`, `av`, `target_id`, `__bid`, `fb_dtsg`, `jazoest`, `lsd`, `Cookie`, and `User-Agent`).
- **Resumable Chunked Video Upload**: Implements Meta's multi-step video upload workflow:
  1. `init_upload`: Initializes composer upload session and retrieves upload parameters.
  2. `requests/start`: Registers file size, extension, and retrieves `upload_session_id` and byte offsets.
  3. `rupload`: Streams chunked binary data (`application/octet-stream`) to `rupload.facebook.com`.
  4. `requests/receive`: Commits upload partition offsets and verifies integrity.
- **Reel & Story Publishing**: Publishes video posts, reels, and stories directly via GraphQL (`BusinessComposerStoryCreationMutation`) and direct Graph Video API.
- **Video & Post Scheduling**: Schedule posts from 10 minutes to 75 days in advance using `scheduled_publish_time`.
- **Default Auto-Translation & AI Dubbing**: Multi-language video translation enabled by default across 14 languages (`translatedAudioMetadata`). See [Scheduling & Auto-Translation Guide](../../docs/SCHEDULING_AND_AUTO_TRANSLATE_GUIDE.md).
- **Scopes & Account Discovery**: Inspects connected Facebook Pages and business accounts.
- **Embedded Web UI**: Self-contained, responsive dashboard served at `GET /custom/facebook/ui`.
- **Clean Architecture & Zero Rust Dependency**: Entirely decoupled in `addons/facebook-suite/`, adhering to inward dependency rules and module ceilings ($\le 450$ lines).

---

## 2. File Organization (Clean Architecture)

```text
addons/facebook-suite/
├── module.json                # Declarative manifest (namespace: /custom/facebook)
├── run.sh                     # Executable launcher (chmod +x)
├── adapter.py                 # Interface adapter for stdio-stream JSON protocol
├── config.json                # Local session credentials and state (git-ignored)
├── README.md                  # This documentation
├── facebook_suite/            # Concentric architecture package
│   ├── __init__.py            # Facade re-exports
│   ├── domain.py              # Pure domain entities, HAR parser, and jazoest derivation
│   ├── client.py              # Application service: HTTP client and GraphQL executor
│   ├── upload.py              # Application service: Chunked video upload orchestrator
│   └── ui.py                  # Presentation layer: Responsive HTML5/CSS/JS dashboard
└── tests/                     # Unit & integration test suite
    ├── test_domain.py         # HAR extraction and domain rule tests
    ├── test_client.py         # Application client and masking tests
    ├── test_upload.py         # Video upload session and offset tests
    └── test_adapter.py        # Protocol adapter route dispatch tests
```

---

## 3. Endpoints & API Reference

All routes are mounted under `/custom/facebook/*`:

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/custom/facebook/` or `/ui` | Embedded HTML5 Web UI configuration page |
| `GET` | `/custom/facebook/v1/status` | Current module status and masked credentials |
| `GET` | `/custom/facebook/v1/config` | View active module configuration |
| `POST` | `/custom/facebook/v1/config` | Update module configuration manually |
| `POST` | `/custom/facebook/v1/config/har` | Upload `.har` capture to auto-extract credentials |
| `GET` | `/custom/facebook/v1/scopes` | List accessible Facebook Pages and identities |
| `POST` | `/custom/facebook/v1/upload/init` | Initialize video upload with Meta |
| `POST` | `/custom/facebook/v1/upload/start` | Start upload session and get chunk offsets |
| `POST` | `/custom/facebook/v1/upload/chunk` | Upload binary chunk (base64) for active session |
| `POST` | `/custom/facebook/v1/upload/video` | One-shot upload and publish video pipeline |
| `POST` | `/custom/facebook/v1/upload/finish` | Finalize video upload |
| `POST` | `/custom/facebook/v1/publish` | Publish Video Reel, Story, or Post |
| `GET` | `/custom/facebook/v1/models` | OpenAI-compatible models endpoint |
| `POST` | `/custom/facebook/v1/chat/completions` | OpenAI-compatible chat completions interface |

---

## 4. HAR Import & Credential Extraction

To authenticate with Meta Business Suite without manual API token generation:
1. Open **Meta Business Suite** (`business.facebook.com`) in your browser.
2. Open DevTools (`F12`), switch to the **Network** tab.
3. Perform a video or reel upload draft.
4. Export the network traffic as a HAR file (`download-facebook.har`).
5. Upload the file to the embedded UI at `http://127.0.0.1:28130/custom/facebook/ui` or submit via curl:

```bash
curl -X POST http://127.0.0.1:28130/custom/facebook/v1/config/har \
  -H "Content-Type: application/json" \
  --data-binary @target/download-facebook.har
```

The adapter extracts `user_id`, `actor_id`, `target_id`, `__bid`, `fb_dtsg`, `lsd`, and computes the valid `jazoest` checksum automatically.

---

## 5. Testing

Run the test suite:

```bash
PYTHONPATH=addons/facebook-suite python3 -m unittest discover -s addons/facebook-suite/tests
```
