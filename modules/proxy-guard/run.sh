#!/bin/sh
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# 1. Prefer precompiled standalone native Rust binary
if [ -f "$SCRIPT_DIR/proxy-guard" ] && [ ! -x "$SCRIPT_DIR/proxy-guard" ]; then
    chmod +x "$SCRIPT_DIR/proxy-guard" 2>/dev/null
fi

if [ -x "$SCRIPT_DIR/proxy-guard" ]; then
    exec "$SCRIPT_DIR/proxy-guard" "$@"
fi

# 2. Prefer target/release binary
if [ -f "$SCRIPT_DIR/target/release/proxy-guard" ] && [ ! -x "$SCRIPT_DIR/target/release/proxy-guard" ]; then
    chmod +x "$SCRIPT_DIR/target/release/proxy-guard" 2>/dev/null
fi

if [ -x "$SCRIPT_DIR/target/release/proxy-guard" ]; then
    exec "$SCRIPT_DIR/target/release/proxy-guard" "$@"
fi

# 3. Build and execute via cargo if available
if command -v cargo >/dev/null 2>&1; then
    exec cargo run --release --manifest-path "$SCRIPT_DIR/Cargo.toml" -- "$@"
fi

echo '{"status": 500, "body": "{\"error\": \"Proxy Guard native Rust binary (proxy-guard) not found and cargo is not installed\"}"}' >&2
exit 1
