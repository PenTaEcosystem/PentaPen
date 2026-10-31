#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
export GSETTINGS_SCHEMA_DIR="$HOME/.local/share/glib-2.0/schemas:${GSETTINGS_SCHEMA_DIR}"
export XDG_DATA_DIRS="$HOME/.local/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

exec "$DIR/target/debug/rnote" "$@"
