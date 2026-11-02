#!/bin/sh
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# 1. Prefer precompiled standalone native Rust binary
if [ -f "$SCRIPT_DIR/daily-news" ] && [ ! -x "$SCRIPT_DIR/daily-news" ]; then
    chmod +x "$SCRIPT_DIR/daily-news" 2>/dev/null
fi

if [ -x "$SCRIPT_DIR/daily-news" ]; then
    exec "$SCRIPT_DIR/daily-news" "$@"
fi

# 2. Prefer target/release binary
if [ -f "$SCRIPT_DIR/target/release/daily-news" ] && [ ! -x "$SCRIPT_DIR/target/release/daily-news" ]; then
    chmod +x "$SCRIPT_DIR/target/release/daily-news" 2>/dev/null
fi

if [ -x "$SCRIPT_DIR/target/release/daily-news" ]; then
    exec "$SCRIPT_DIR/target/release/daily-news" "$@"
fi

# 3. Build and execute via cargo if available
if command -v cargo >/dev/null 2>&1; then
    exec cargo run --release --manifest-path "$SCRIPT_DIR/Cargo.toml" -- "$@"
fi

echo '{"status": 500, "body": "{\"error\": \"Daily News native Rust binary (daily-news) not found and cargo is not installed\"}"}'
exit 1
