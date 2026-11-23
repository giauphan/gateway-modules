# Rust Safety & Anti-Pattern Rules

Reference detail for section 6 of [`AGENTS.md`](../AGENTS.md). Loaded on demand only — not part of the always-on agent context.

---

## 1. `unwrap()` in production

In Rust, **`unwrap()`** extracts the value inside an `Option` or `Result`. On `Some`/`Ok` it returns the value; on `None`/`Err` it **panics** and aborts the current execution flow.

While learning Rust or prototyping, `unwrap()` avoids writing error-handling code. In production, a single `unwrap()` that meets unexpected data takes down the whole service.

```rust
use std::fs::File;

fn main() {
    // 1. With Option
    let items = vec![10, 20, 30];
    let first = items.first().unwrap(); // Safe because the vec is not empty -> 10

    let empty: Vec<i32> = vec![];
    // The line below would PANIC: "called `Option::unwrap()` on a `None` value"
    // let err = empty.first().unwrap();

    // 2. With Result
    // If the file does not exist the program PANICS immediately
    let file = File::open("config.json").unwrap();
}
```

### Safe replacements for `unwrap()`

| Method | When to use | Behaviour on `None` / `Err` |
| :--- | :--- | :--- |
| **`?` operator** | Function returns `Result` or `Option` | Returns the error to the caller (early return) |
| **`unwrap_or(val)`** | A static default value exists | Returns `val` |
| **`unwrap_or_else(fn)`** | Default needs computation | Runs the closure only on failure (lazy evaluation) |
| **`unwrap_or_default()`** | Type implements `Default` | Returns the type default (`0`, empty string, empty vec) |
| **`match` / `if let`** | Error branch handled locally | Executes that branch |
| **`expect("msg")`** | Provably infallible (startup / tests) | Still panics, but prints a clear cause |

```rust
use std::fs::File;
use std::io::{self, Read};

// GOOD: `?` forwards the error to the caller
fn load_config(path: &str) -> Result<String, io::Error> {
    let mut file = File::open(path)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}

fn handle(input: Option<i32>) {
    // GOOD: provide a fallback
    let value = input.unwrap_or(0);

    // GOOD: handle only when data is present
    if let Some(v) = input {
        println!("received: {v}");
    }
}
```

---

## 2. Lifetimes (`'a`) and `Cow<'a, B>` instead of needless `.clone()`

When the borrow checker complains, **never** blindly duplicate data with `.clone()`. Apply the **Lifetimes** / **`std::borrow::Cow` (Clone-on-Write)** decision matrix:

| Situation | Required solution | Heap cost |
| :--- | :--- | :--- |
| Read-only function | Borrowed refs: `&str`, `&[T]`, `&T` | **0 (zero-cost)** |
| Struct/view slicing existing data | **Lifetime `'a`** (`&'a str`, `&'a [T]`, `struct View<'a>`) | **0 (zero-copy)** |
| Mostly read (fast path), occasionally modified/rebuilt | **`Cow<'a, str>` / `Cow<'a, [T]>`** | **0 while unchanged; allocate only on mutation** |
| Accept `&'static str`, `&str` and `String` uniformly | **`Cow<'a, str>`** or `Into<Cow<'a, str>>` | **0 when passed a `&str`** |
| Immutable data shared across threads / async tasks (`'static`) | `Arc<str>` / `Arc<[T]>` | **One allocation; clone only bumps the refcount** |

### A. Lifetimes (`'a`) — direct borrowing (zero-copy)

- **Essence**: lifetime `'a` is a lifetime label letting the compiler prove a reference (`&'a T`) never outlives its data (no dangling pointer) without copying data to the heap.
- **When to apply**:
  1. **Zero-copy parser / view struct** — when the struct only reads/slices an input buffer (log line, JSON/text payload, config):

     ```rust
     // GOOD: zero-copy struct with lifetime 'a (0 heap allocation)
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
  2. **Function returning a reference tied to several inputs** — name the lifetime explicitly (`fn longest<'a>(x: &'a str, y: &'a str) -> &'a str`).
  3. **When NOT to force a lifetime** — if the data is always transformed or the struct must outlive the source buffer (e.g. sent through a channel to another worker thread), do not over-constrain `'a`; use `Cow<'static, str>`, `Arc<str>`, or `String`.

### B. `std::borrow::Cow<'a, B>` (Clone-on-Write) — clone only on write

- **Essence**: `Cow<'a, B>` is a smart-pointer enum with two variants:
  - `Cow::Borrowed(&'a B)`: borrows the original data (no allocation).
  - `Cow::Owned(<B as ToOwned>::Owned)`: owns heap data (`String` for `str`, `Vec<T>` for `[T]`) — **allocated only on modification**.
- **3 required patterns**:
  1. **Conditional normalization / sanitization**:

     ```rust
     use std::borrow::Cow;

     pub fn sanitize_identifier(input: &str) -> Cow<'_, str> {
         // Fast path (90%+ of cases): no space -> Borrowed (0 allocation)
         if !input.contains(' ') {
             return Cow::Borrowed(input);
         }
         // Slow path: allocate a new String only when replacement is needed
         Cow::Owned(input.replace(' ', "_"))
     }
     ```
  2. **Lazy mutation with `to_mut()`**:
     - **Cardinal rule**: always test conditions on the borrowed form first, and call `.to_mut()` only at the exact point mutation is certain (`.to_mut()` clones immediately while `Borrowed`).
     ```rust
     use std::borrow::Cow;

     pub fn ensure_https(url: &str) -> Cow<'_, str> {
         let mut cow = Cow::Borrowed(url);
         if !cow.starts_with("https://") {
             // Clone into a String only when the https:// prefix is missing
             cow.to_mut().insert_str(0, "https://");
         }
         cow
     }
     ```
  3. **Config structs / DTOs (and zero-copy serde)**:
     - **Important serde note**: by default `#[derive(Deserialize)]` on `Cow<'a, str>` still allocates `Cow::Owned(String)`. For zero-copy DTO/config deserialization, **`#[serde(borrow)]` is mandatory**:
     ```rust
     use std::borrow::Cow;
     use serde::Deserialize;

     #[derive(Deserialize)]
     pub struct EndpointConfig<'a> {
         #[serde(borrow)] // Required so serde borrows directly from the input buffer (Cow::Borrowed)
         pub host: Cow<'a, str>, // Accepts &'a str (0 alloc) or unescapes to String when the JSON has escapes (\n, \")
         pub port: u16,
     }
     ```

---

## 3. Other anti-patterns to avoid

Besides `unwrap()`, these common habits hurt performance or safety in Rust:

### 3.1 `.clone()` to get past the borrow checker

Cloning to satisfy the borrow checker allocates heap memory needlessly and gives up Rust's speed advantage.

```rust
// AVOID: clone the whole String just to read it
fn print_bad(name: String) {
    println!("hello: {name}");
}
// Call site: print_bad(name.clone()); -> wasted memory

// PREFER: take a reference (&str)
fn print_good(name: &str) {
    println!("hello: {name}");
}
// Call site: print_good(&name); -> zero-cost copy
```

### 3.2 `&String` or `&Vec` parameters

`&String` / `&Vec` narrows the accepted input. `&str` / slice `&[T]` accepts `String`, `&String`, `&'static str`, arrays, and `Vec`.

```rust
// AVOID
fn sum_bad(values: &Vec<i32>) -> i32 {
    values.iter().sum()
}

// PREFER: slice &[i32] is more flexible
fn sum_good(values: &[i32]) -> i32 {
    values.iter().sum()
}
```

### 3.3 Silently dropping errors with `let _ = ...`

Calling a `Result`-returning function without using it triggers `unused Result`. Assigning `let _ = fallible();` silences the warning **and swallows the error**, making bugs very hard to find.

```rust
// AVOID: the failure is silently lost
let _ = std::fs::remove_file("temp.txt");

// PREFER: at least log the failure
if let Err(e) = std::fs::remove_file("temp.txt") {
    eprintln!("warning: could not remove temp file: {e}");
}
```

### 3.4 Looping by index instead of iterator

`0..vec.len()` plus `vec[i]` forces a bounds check per iteration and can panic if the index is miscounted.

```rust
let items = vec![1, 2, 3, 4, 5];

// AVOID: slower, risk of index out of bounds
for i in 0..items.len() {
    println!("{}", items[i]);
}

// PREFER: iterator (the compiler elides bounds checks)
for item in &items {
    println!("{item}");
}

// When the position is needed, use .enumerate()
for (i, item) in items.iter().enumerate() {
    println!("index {i}: {item}");
}
```

### 3.5 String concatenation with `+` in a loop

`+` takes ownership of the left operand and reallocates repeatedly when used in a loop.

```rust
let words = vec!["Rust", "safe", "and", "fast"];

// AVOID: manual concatenation in a loop
let mut sentence = String::new();
for word in &words {
    sentence = sentence + word + " "; // reallocates continuously
}

// PREFER: .join() or push_str()
let sentence_ok = words.join(" ");
```

### 3.6 `unsafe` to bypass the compiler

`unsafe` disables Rust's memory-safety checks. Use it only for C FFI or low-level data structures that have been carefully verified — never to "get around" borrow/lifetime errors.

---

> **Automated tip:** run `cargo clippy` in the project. It detects unsafe `unwrap()` and the anti-patterns above, and suggests fixes directly in the terminal.

---

## 4. Advanced AI Rust code generation rules

When generating or refactoring Rust, these architectural and optimization rules are mandatory.

### 4.1 Clean / Hexagonal Architecture in Rust (ports & adapters)

- **Separate layers with a Cargo workspace or module visibility (`pub(crate)`)**:
  - `domain`: entities, value objects, domain events and errors. **Zero peripheral dependencies** (no `tokio`, `sqlx`, `axum`, `reqwest`).
  - `application` (use cases): orchestration and **inbound/outbound ports (traits)**. Depends only on `domain`.
  - `adapters` / `infrastructure`: HTTP handlers (Axum/Actix/std::net), repositories (SQLx/Postgres/Redis), CLI, and `main.rs` (composition root). Implements the `application` traits.
- **Avoid anemic domain models and DTO duplication hell**:
  - Keep behaviour and invariants inside the entity/value object instead of pushing everything into external procedures.
  - Only create a separate DTO at the boundary (HTTP/DB adapter) when its schema genuinely differs from the domain model.

### 4.2 Type-driven design and "parse, don't validate"

- **Kill primitive obsession with the newtype pattern**:
  - Do not pass raw `String`, `u64`, `i32` across business layers for meaningful values (such as `UserId`, `OrgId`, `Email`, `Money`).
  - Wrap them in a tuple struct with a **private** field, constructible only through `parse` / `TryFrom` returning `Result` (**correct by construction**):
    ```rust
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct EmailAddress(String); // inner field is private

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
- **Typestate pattern (turn invalid states into compile errors)**:
  - Encode object lifecycle state in the type system (`struct Connection<State>`) instead of a runtime `bool` flag (`is_connected: bool`).

### 4.3 Layered error handling (`thiserror` vs `anyhow`)

- **`domain` & `application` (library crates)**: **`thiserror`** is mandatory — explicit error `enum`s that callers can `match`. Infrastructure errors (`sqlx::Error`, `io::Error`) must never leak into a `domain` function signature.
- **`adapters` / `bin` (`main.rs`, CLI, top-level handlers)**: use **`anyhow::Result`** with `.context("clear error context")?` to aggregate traces for logging or the endpoint response.

### 4.4 Performance, memory and concurrency (zero-cost & concurrency)

1. **Preallocate (`with_capacity`) and reuse buffers**:
   - When the size is known in advance, use `Vec::with_capacity(n)`, `String::with_capacity(n)`, `HashMap::with_capacity(n)`.
   - In hot loops, accept `&mut Vec<u8>` / `&mut String` and call `.clear()` instead of allocating per iteration.
2. **Static vs dynamic dispatch**:
   - Prefer generics (`fn execute<R: UserRepository>(repo: &R)`) on hot paths so the compiler monomorphizes and inlines (zero-cost).
   - Use `Arc<dyn UserRepository + Send + Sync>` at the application composition root when compile time and configuration flexibility matter.
3. **Async and lock safety (deadlock prevention)**:
   - **Never hold a synchronous lock guard (`std::sync::MutexGuard`, `RwLockReadGuard`) across `.await`**. Lock, extract the data, or `drop(guard)` inside a `{ ... }` block before awaiting.
4. **Strict lint configuration (`Cargo.toml`)**:
   Configure the compiler to reject these bug classes:
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