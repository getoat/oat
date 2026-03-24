#!/usr/bin/env bash
set -euo pipefail

version=${1:-}
if [[ -z "$version" ]]; then
  echo "Usage: $0 <version>" >&2
  exit 1
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
manifest_path="$repo_root/Cargo.toml"
tmp_file=$(mktemp)

awk -v version="$version" '
BEGIN {
  updated = 0
}
!updated && /^version = "/ {
  sub(/^version = "[^"]+"/, "version = \"" version "\"")
  updated = 1
}
{
  print
}
END {
  if (!updated) {
    exit 1
  }
}
' "$manifest_path" >"$tmp_file"

mv "$tmp_file" "$manifest_path"
