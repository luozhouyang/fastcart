#!/usr/bin/env bash
#
# Verify a Fastcart store database backup without modifying it.
#
# Usage:
#   scripts/verify-backup.sh /path/to/store.sqlite
#
# Exit codes:
#   0  the backup is a readable SQLite database with a schema and a clean
#      integrity check
#   1  the backup is missing, empty, corrupt or has no schema
#   2  usage error or a required tool is unavailable
#
set -euo pipefail

usage() {
  echo "usage: $0 <backup-file>" >&2
}

die() {
  echo "verify-backup: $*" >&2
  exit 1
}

if [[ $# -ne 1 ]]; then
  usage
  exit 2
fi

backup="$1"

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "verify-backup: sqlite3 is required" >&2
  exit 2
fi

if [[ ! -e "$backup" ]]; then
  die "file does not exist: $backup"
fi

if [[ ! -f "$backup" ]]; then
  die "not a regular file: $backup"
fi

if [[ ! -s "$backup" ]]; then
  die "file is empty: $backup"
fi

integrity="$(sqlite3 -readonly "$backup" "PRAGMA integrity_check;")"
if [[ "$integrity" != "ok" ]]; then
  die "integrity check failed: $integrity"
fi

table_count="$(sqlite3 -readonly "$backup" "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table';")"
if [[ "$table_count" -le 0 ]]; then
  die "backup contains no schema objects"
fi

migration_version="$(sqlite3 -readonly "$backup" \
  "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1;" 2>/dev/null || echo 0)"

echo "ok: $backup"
echo "  tables: ${table_count}"
echo "  migration version: ${migration_version}"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$backup"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$backup"
fi
