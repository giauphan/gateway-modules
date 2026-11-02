#!/bin/sh
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# 1. Prefer native compiled Rust binary if available (Python-free mode)
if [ -x "$SCRIPT_DIR/rust/target/release/daily-news" ]; then
    exec "$SCRIPT_DIR/rust/target/release/daily-news" "$@"
fi
if [ -x "$SCRIPT_DIR/daily-news" ]; then
    exec "$SCRIPT_DIR/daily-news" "$@"
fi

# 2. Fall back to Python adapter if Python 3 is installed
if command -v python3 >/dev/null 2>&1; then
    exec python3 "$SCRIPT_DIR/adapter.py" "$@"
fi

echo '{"status": 500, "body": "{\"error\": \"Neither native Rust binary (rust/target/release/daily-news) nor python3 runtime was found\"}"}'
exit 1
