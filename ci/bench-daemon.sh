#!/usr/bin/env bash
# Run from any directory; Python manages isolated pinned checkouts and restores every edit.
set -euo pipefail
exec python3 "$(dirname "$0")/bench_daemon.py" "$@"
