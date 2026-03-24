#!/usr/bin/env bash
set -euo pipefail

version=${1:-}
if [[ -z "$version" ]]; then
  echo "Usage: $0 <version>" >&2
  exit 1
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
manifest_path="$repo_root/Cargo.toml"
lockfile_path="$repo_root/Cargo.lock"
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

tmp_file=$(mktemp)

awk -v version="$version" '
BEGIN {
  in_package = 0
  is_root_package = 0
  updated = 0
}
/^\[\[package\]\]$/ {
  in_package = 1
  is_root_package = 0
  print
  next
}
in_package && /^name = "oat"$/ {
  is_root_package = 1
  print
  next
}
in_package && is_root_package && /^version = "/ {
  sub(/^version = "[^"]+"/, "version = \"" version "\"")
  updated = 1
  print
  next
}
{
  print
}
END {
  if (!updated) {
    exit 1
  }
}
' "$lockfile_path" >"$tmp_file"

mv "$tmp_file" "$lockfile_path"
