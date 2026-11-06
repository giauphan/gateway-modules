# Agent Rules for Toolcase Gateway Modules (`GEMINI.md` / `AGENTS.md`)

This document establishes the architecture, language, and quality standards for all feature modules and extensions in this repository. It mirrors [`AGENTS.md`](./AGENTS.md).

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

---

## 6. Rust Safety & Anti-Pattern Rules (Quy tắc An toàn & Tránh Anti-pattern trong Rust)

Trong Rust, **`unwrap()`** là phương thức dùng để lấy giá trị bên trong kiểu `Option` hoặc `Result`. Nếu giá trị tồn tại (`Some` hoặc `Ok`), hàm trả về giá trị đó; ngược lại, nếu gặp `None` hoặc `Err`, chương trình sẽ ngay lập tức dừng đột ngột (**panic**) và làm sập luồng thực thi hiện tại.

### 6.1. Cách hoạt động của `unwrap()` và rủi ro trong Production

Khi mới học Rust hoặc viết code thử nghiệm (prototype), `unwrap()` giúp lấy dữ liệu nhanh mà không cần viết logic xử lý lỗi. Tuy nhiên, trong môi trường thực tế (production), một lệnh `unwrap()` gặp dữ liệu ngoài dự kiến sẽ làm sập toàn bộ dịch vụ.

```rust
use std::fs::File;

fn main() {
    // 1. Dùng với Option
    let danh_sach = vec![10, 20, 30];
    let phan_tu_dau = danh_sach.first().unwrap(); // An toàn vì vec không rỗng -> 10
    
    let vec_rong: Vec<i32> = vec![];
    // Dòng dưới sẽ gây PANIC: "called `Option::unwrap()` on a `None` value"
    // let loi = vec_rong.first().unwrap(); 

    // 2. Dùng với Result
    // Nếu file không tồn tại, chương trình sẽ PANIC ngay lập tức
    let file = File::open("config.json").unwrap();
}
```

#### Các cách thay thế `unwrap()` an toàn

Thay vì dùng `unwrap()`, hãy chọn phương pháp phù hợp với ngữ cảnh:

| Phương pháp | Khi nào nên dùng | Hành vi khi gặp `None` / `Err` |
| :--- | :--- | :--- |
| **Toán tử `?`** | Trong hàm trả về `Result` hoặc `Option` | Tự động trả lỗi về hàm gọi nó (early return) |
| **`unwrap_or(val)`** | Có sẵn giá trị mặc định tĩnh | Trả về giá trị mặc định `val` |
| **`unwrap_or_else(fn)`** | Giá trị mặc định cần tính toán phức tạp | Chỉ chạy hàm closure khi thực sự có lỗi (lazy evaluation) |
| **`unwrap_or_default()`** | Kiểu dữ liệu có triển khai trait `Default` | Trả về mặc định của kiểu (ví dụ: `0`, chuỗi rỗng, vector rỗng) |
| **`match` / `if let`** | Cần xử lý nhánh lỗi ngay tại chỗ | Thực thi khối code xử lý riêng biệt |
| **`expect("msg")`** | Chắc chắn 100% không thể lỗi (hoặc khi khởi tạo app) | Vẫn gây panic nhưng in ra thông báo rõ nguyên nhân |

```rust
use std::fs::File;
use std::io::{self, Read};

// TỐT: Dùng toán tử `?` để chuyển tiếp lỗi cho nơi gọi xử lý
fn doc_cau_hinh(duong_dan: &str) -> Result<String, io::Error> {
    let mut file = File::open(duong_dan)?;
    let mut noi_dung = String::new();
    file.read_to_string(&mut noi_dung)?;
    Ok(noi_dung)
}

fn xu_ly_du_lieu(input: Option<i32>) {
    // TỐT: Cung cấp giá trị dự phòng
    let so = input.unwrap_or(0);

    // TỐT: Chỉ xử lý khi có dữ liệu bằng if let
    if let Some(gia_tri) = input {
        println!("Giá trị nhận được: {}", gia_tri);
    }
}
```

---

### Giải pháp dùng Lifetimes (`'a`) và `Cow<'a, B>` để tránh `.clone()` không cần thiết (Bắt buộc khi Generate Code Rust)

Khi trình biên dịch báo lỗi mượn vùng nhớ (borrow checker), **tuyệt đối không** nhân bản dữ liệu bằng `.clone()` một cách mù quáng. Hãy áp dụng **Lifetimes** và **`std::borrow::Cow` (Clone-on-Write)** theo ma trận quyết định sau:

| Tình huống khi sinh code Rust | Giải pháp bắt buộc | Chi phí cấp phát (Heap) |
| :--- | :--- | :--- |
| Hàm chỉ đọc dữ liệu | Tham chiếu mượn: `&str`, `&[T]`, `&T` | **0 (Zero-cost)** |
| Struct / Hàm trích xuất view/slice từ dữ liệu gốc có sẵn | **Lifetime `'a`** (`&'a str`, `&'a [T]`, `struct View<'a>`) | **0 (Zero-copy)** |
| Đa số chỉ đọc (Fast path), đôi khi mới cần sửa đổi/tạo mới (Slow path) | **`Cow<'a, str>` / `Cow<'a, [T]>`** | **0 khi giữ nguyên, chỉ cấp phát khi sửa** |
| Hàm/Struct cần nhận linh hoạt cả chuỗi tĩnh `&'static str`, `&str` lẫn `String` | **`Cow<'a, str>`** hoặc `Into<Cow<'a, str>>` | **0 nếu truyền `&str`** |
| Chia sẻ dữ liệu bất biến qua nhiều thread / async task (`'static`) | `Arc<str>` / `Arc<[T]>` | **1 lần cấp phát, clone chỉ tăng ref-count** |

#### A. Cách dùng Lifetimes (`'a`) — Mượn trực tiếp (Zero-Copy Borrowing)
- **Bản chất**: Lifetime `'a` là nhãn thời gian sống giúp trình biên dịch đảm bảo tham chiếu (`&'a T`) không bao giờ sống lâu hơn dữ liệu gốc (tránh dangling pointer) mà không cần copy dữ liệu lên Heap.
- **Khi nào áp dụng**:
  1. **Zero-copy Parser / View Struct**: Khi struct chỉ đóng vai trò đọc/bóc tách từ một buffer đầu vào (log line, JSON/text payload, config):
     ```rust
     // NÊN DÙNG: Zero-copy struct với Lifetime 'a (0 heap allocation)
     pub struct LogEntry<'a> {
         pub level: &'a str,
         pub message: &'a str,
     }

     impl<'a> LogEntry<'a> {
         pub fn parse(line: &'a str) -> Option<Self> {
             let (level, message) = line.split_once(": ")?;
             Some(Self { level, message })
         }
     }
     ```
  2. **Hàm trả về tham chiếu phụ thuộc nhiều đầu vào**: Khai báo rõ `'a` để gắn kết thời gian sống của giá trị trả về với tham số đầu vào tương ứng (`fn longest<'a>(x: &'a str, y: &'a str) -> &'a str`).
  3. **Khi nào KHÔNG ép dùng Lifetime**: Nếu dữ liệu luôn bị biến đổi hoặc struct phải sống độc lập vượt khỏi scope của buffer gốc (ví dụ gửi qua channel sang worker thread khác), đừng cố buộc `'a` quá rộng — hãy dùng `Cow<'static, str>`, `Arc<str>`, hoặc `String`.

#### B. Cách dùng `std::borrow::Cow<'a, B>` (Clone-on-Write) — Chỉ Clone khi thực sự ghi/sửa
- **Bản chất**: `Cow<'a, B>` là một smart pointer enum gồm 2 biến thể:
  - `Cow::Borrowed(&'a B)`: Mượn dữ liệu gốc (không cấp phát bộ nhớ).
  - `Cow::Owned(<B as ToOwned>::Owned)`: Sở hữu dữ liệu trên heap (`String` đối với `str`, `Vec<T>` đối với `[T]`) — **chỉ cấp phát khi có thao tác sửa đổi**.
- **3 Mẫu thiết kế (Patterns) bắt buộc khi sinh code Rust**:
  1. **Chuẩn hóa / Làm sạch dữ liệu có điều kiện (Conditional Transformation)**:
     ```rust
     use std::borrow::Cow;

     pub fn sanitize_identifier(input: &str) -> Cow<'_, str> {
         // Fast path (90%+ trường hợp): không có khoảng trắng -> trả về Borrowed (0 allocation)
         if !input.contains(' ') {
             return Cow::Borrowed(input);
         }
         // Slow path: chỉ cấp phát String mới khi thực sự cần thay thế
         Cow::Owned(input.replace(' ', "_"))
     }
     ```
  2. **Sửa đổi lười với `to_mut()` (Lazy Mutation)**:
     - **Quy tắc sống còn**: Luôn kiểm tra điều kiện trên bản mượn trước, **chỉ gọi `.to_mut()` ngay tại thời điểm chắc chắn cần sửa đổi** (vì `.to_mut()` sẽ lập tức clone nếu đang ở trạng thái `Borrowed`).
     ```rust
     use std::borrow::Cow;

     pub fn ensure_https(url: &str) -> Cow<'_, str> {
         let mut cow = Cow::Borrowed(url);
         if !cow.starts_with("https://") {
             // Chỉ clone thành String khi thiếu tiền tố https://
             cow.to_mut().insert_str(0, "https://");
         }
         cow
     }
     ```
  3. **Struct cấu hình / DTO kết hợp dữ liệu tĩnh và động (và Zero-Copy Serde)**:
     - **Lưu ý quan trọng với Serde**: Mặc định `#[derive(Deserialize)]` trên `Cow<'a, str>` vẫn cấp phát `Cow::Owned(String)`. Khi sinh code Rust cho DTO/Config cần zero-copy deserialization, **bắt buộc** thêm `#[serde(borrow)]`:
     ```rust
     use std::borrow::Cow;
     use serde::Deserialize;

     #[derive(Deserialize)]
     pub struct EndpointConfig<'a> {
         #[serde(borrow)] // Bắt buộc để Serde mượn trực tiếp từ input buffer (Cow::Borrowed)
         pub host: Cow<'a, str>, // Có thể nhận &'a str (0 alloc) hoặc tự unescape sang String khi JSON có ký tự escape (\n, \")
         pub port: u16,
     }
     ```

---

### 6.2. Những đoạn code cần tránh dùng trong Rust (Anti-patterns)

Ngoài `unwrap()`, dưới đây là những thói quen viết code phổ biến gây giảm hiệu năng hoặc mất an toàn trong Rust:

#### 1. Lạm dụng `.clone()` để vượt qua Borrow Checker
Khi trình biên dịch báo lỗi mượn vùng nhớ (borrow checker), nhiều lập trình viên nhân bản dữ liệu bằng `.clone()` để sửa lỗi nhanh. Điều này làm cấp phát bộ nhớ heap không cần thiết và làm mất đi lợi thế tốc độ của Rust.

```rust
// TRÁNH DÙNG: Clone toàn bộ chuỗi chỉ để đọc
fn in_ten_xau(ten: String) {
    println!("Xin chào: {}", ten);
}
// Khi gọi: in_ten_xau(ten_goc.clone()); -> Tốn bộ nhớ

// NÊN DÙNG: Truyền tham chiếu (&str)
fn in_ten_tot(ten: &str) {
    println!("Xin chào: {}", ten);
}
// Khi gọi: in_ten_tot(&ten_goc); -> Không tốn chi phí copy (Zero-cost)
```

#### 2. Dùng `&String` hoặc `&Vec` làm tham số hàm
Truyền `&String` hoặc `&Vec` làm hàm bị bó hẹp kiểu đầu vào. Nếu dùng `&str` hoặc slice `&[T]`, hàm có thể nhận cả `String`, `&String`, chuỗi tĩnh `&'static str`, mảng tĩnh lẫn `Vec`.

```rust
// TRÁNH DÙNG
fn tinh_tong_xau(danh_sach: &Vec<i32>) -> i32 {
    danh_sach.iter().sum()
}

// NÊN DÙNG: Dùng slice &[i32] linh hoạt hơn
fn tinh_tong_tot(danh_sach: &[i32]) -> i32 {
    danh_sach.iter().sum()
}
```

#### 3. Bỏ qua lỗi ngầm bằng `let _ = ...`
Khi gọi một hàm trả về `Result` mà không dùng kết quả, Rust sẽ cảnh báo `unused Result`. Việc gán `let _ = ham_co_the_loi();` sẽ tắt cảnh báo nhưng đồng thời **nuốt luôn lỗi**, khiến việc tìm bug cực kỳ khó khăn.

```rust
// TRÁNH DÙNG: Lỗi bị bỏ qua hoàn toàn mà không ai biết
let _ = std::fs::remove_file("tam_thoi.txt");

// NÊN DÙNG: Ít nhất hãy ghi log nếu việc xóa thất bại
if let Err(e) = std::fs::remove_file("tam_thoi.txt") {
    eprintln!("Cảnh báo: Không thể xóa file tạm: {}", e);
}
```

#### 4. Duyệt mảng bằng chỉ số (Index) thay vì Iterator
Duyệt mảng qua chỉ số `0..vec.len()` rồi truy cập `vec[i]` buộc Rust phải kiểm tra giới hạn mảng (bounds checking) ở mỗi vòng lặp và dễ gây panic nếu tính sai chỉ số.

```rust
let danh_sach = vec![1, 2, 3, 4, 5];

// TRÁNH DÙNG: Chậm hơn và có nguy cơ panic index out of bounds
for i in 0..danh_sach.len() {
    println!("{}", danh_sach[i]);
}

// NÊN DÙNG: Dùng Iterator (nhanh hơn vì trình biên dịch bỏ qua bước kiểm tra giới hạn)
for phan_tu in &danh_sach {
    println!("{}", phan_tu);
}

// Nếu cần cả vị trí (index), dùng .enumerate()
for (i, phan_tu) in danh_sach.iter().enumerate() {
    println!("Vị trí {}: {}", i, phan_tu);
}
```

#### 5. Nối chuỗi bằng toán tử `+` trong vòng lặp
Toán tử `+` trên chuỗi đòi hỏi quyền sở hữu (ownership) của chuỗi đầu tiên và dễ dẫn đến việc cấp phát lại bộ nhớ liên tục nếu dùng không khéo.

```rust
let cac_tu = vec!["Rust", "an", "toàn", "và", "nhanh"];

// TRÁNH DÙNG: Nối chuỗi thủ công trong vòng lặp
let mut cau = String::new();
for tu in &cac_tu {
    cau = cau + tu + " "; // Cấp phát vùng nhớ mới liên tục
}

// NÊN DÙNG: Dùng .join() hoặc push_str()
let cau_tot = cac_tu.join(" ");
```

#### 6. Dùng `unsafe` để lách luật trình biên dịch
Từ khóa `unsafe` tắt các kiểm tra an toàn bộ nhớ của Rust. Chỉ dùng `unsafe` khi giao tiếp với thư viện C (FFI) hoặc viết cấu trúc dữ liệu cấp thấp đã được kiểm chứng kỹ, tuyệt đối không dùng để "vượt rào" khi gặp lỗi mượn biến (borrow/lifetime).

---

> **Mẹo kiểm tra tự động:** Bạn có thể chạy lệnh `cargo clippy` trong dự án Rust. Công cụ này sẽ tự động phát hiện các đoạn code dùng `unwrap()` không an toàn và các anti-pattern kể trên, đồng thời gợi ý cách sửa trực tiếp trên terminal.

---

### Advanced AI Rust Code Generation Rules: Clean Architecture, Type-Driven Design & Performance Optimization

Khi AI sinh code (generate) hoặc refactor dự án Rust, bắt buộc áp dụng các nguyên tắc kiến trúc và tối ưu hóa nâng cao sau:

#### 1. Clean / Hexagonal Architecture trong Rust (Ports & Adapters)
- **Tách biệt tầng bằng Cargo Workspace hoặc Module Visibility (`pub(crate)`)**:
  - `domain`: Chứa Entities, Value Objects, Domain Events và Domain Errors. **0 phụ thuộc ngoại vi** (không `tokio`, `sqlx`, `axum`, `reqwest`).
  - `application` (Use Cases): Chứa logic điều phối nghiệp vụ và định nghĩa **Inbound / Outbound Ports (Traits)**. Chỉ phụ thuộc vào `domain`.
  - `adapters` / `infrastructure`: Chứa HTTP handlers (Axum/Actix/std::net), Database repositories (SQLx/Postgres/Redis), CLI và `main.rs` (Composition Root). Triển khai các `trait` của tầng `application`.
- **Tránh Anemic Domain Model & DTO Duplication Hell**:
  - Đóng gói hành vi và quy tắc bất biến (invariants) ngay trong Entity / Value Object thay vì đẩy hết ra hàm thủ tục bên ngoài.
  - Chỉ tạo DTO riêng ở biên (HTTP/DB Adapter) khi schema thực sự khác biệt với Domain Model.

#### 2. Type-Driven Design & Triết lý "Parse, Don't Validate"
- **Chống Primitive Obsession bằng Newtype Pattern**:
  - Không truyền trực tiếp `String`, `u64`, `i32` trần qua các tầng nghiệp vụ cho các giá trị có ngữ nghĩa (như `UserId`, `OrgId`, `Email`, `Money`).
  - Bọc trong tuple struct với trường `private` và chỉ cho phép khởi tạo qua hàm `parse` / `TryFrom` trả về `Result` (**Correct-by-Construction**):
    ```rust
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct EmailAddress(String); // Trường bên trong là private

    impl EmailAddress {
        pub fn parse(raw: impl Into<String>) -> Result<Self, DomainError> {
            let s = raw.into();
            if s.contains('@') && s.contains('.') {
                Ok(Self(s))
            } else {
                Err(DomainError::InvalidEmail)
            }
        }
        pub fn as_str(&self) -> &str { &self.0 }
    }
    ```
- **Typestate Pattern (Biến trạng thái bất hợp lệ thành lỗi biên dịch)**:
  - Mã hóa trạng thái vòng đời đối tượng vào hệ thống kiểu (`struct Connection<State>`) thay vì dùng cờ `bool` (`is_connected: bool`) kiểm tra lúc runtime.

#### 3. Kiến trúc Xử lý Lỗi Phân Tầng (`thiserror` vs `anyhow`)
- **Tầng `domain` & `application` (Library Crates)**: Bắt buộc dùng **`thiserror`** để định nghĩa `enum` lỗi tường minh, có thể `match` để xử lý nhánh lỗi nghiệp vụ. Không bao giờ để lọt lỗi hạ tầng (`sqlx::Error`, `io::Error`) vào chữ ký hàm của `domain`.
- **Tầng `adapters` / `bin` (`main.rs`, CLI, Top-level Handlers)**: Dùng **`anyhow::Result`** kết hợp `.context("ngữ cảnh lỗi rõ ràng")?` để gom vết lỗi khi ghi log hoặc trả về điểm cuối.

#### 4. Tối ưu hóa Hiệu năng, Bộ nhớ & Đa luồng (Zero-Cost & Concurrency)
1. **Cấp phát trước vùng nhớ (`with_capacity`) & Tái sử dụng Buffer**:
   - Khi biết trước kích thước ước lượng, bắt buộc dùng `Vec::with_capacity(n)`, `String::with_capacity(n)`, `HashMap::with_capacity(n)`.
   - Trong vòng lặp tần suất cao (hot loop), truyền `&mut Vec<u8>` / `&mut String` từ ngoài vào và gọi `.clear()` thay vì khởi tạo mới ở mỗi vòng lặp.
2. **Static Dispatch vs Dynamic Dispatch**:
   - Ưu tiên Generics (`fn execute<R: UserRepository>(repo: &R)`) trên đường truyền nóng (hot path) để trình biên dịch monomorphize và inline hàm (Zero-cost).
   - Dùng `Arc<dyn UserRepository + Send + Sync>` ở tầng khởi tạo ứng dụng (App State / Router) khi cần giảm thời gian biên dịch và linh hoạt cấu hình.
3. **Quy tắc An toàn Async & Lock (Chống Deadlock)**:
   - **Tuyệt đối không giữ khóa đồng bộ (`std::sync::MutexGuard`, `RwLockReadGuard`) băng qua điểm `.await`**. Hãy khóa, trích xuất dữ liệu hoặc thả khóa (`drop(guard)`) trong một block `{ ... }` trước khi gọi `.await`.
4. **Cấu hình Lints Nghiêm ngặt (`Cargo.toml`)**:
   Khuyến nghị cấu hình trong `Cargo.toml` để trình biên dịch tự động chặn lỗi:
   ```toml
   [lints.clippy]
   all = { level = "deny", priority = -1 }
   pedantic = { level = "warn", priority = -1 }
   perf = { level = "deny", priority = -1 }
   unwrap_used = "deny"
   expect_used = "warn"
   panic = "deny"
   todo = "warn"
   dbg_macro = "deny"
   ```
