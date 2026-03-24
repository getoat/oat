#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_root"

if [[ -z "${GITHUB_OUTPUT:-}" ]]; then
  echo "GITHUB_OUTPUT must be set" >&2
  exit 1
fi

branch_name=${GITHUB_REF_NAME:-$(git rev-parse --abbrev-ref HEAD)}
manifest_version=$(grep -m1 '^version = "' Cargo.toml | cut -d '"' -f 2)

if [[ ! "$manifest_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Unsupported Cargo manifest version: $manifest_version" >&2
  exit 1
fi

latest_stable_tag=$(
  git tag --merged HEAD --list 'v*' --sort=-version:refname \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | head -n 1 \
    || true
)

baseline_version=${latest_stable_tag#v}
if [[ -z "$baseline_version" ]]; then
  baseline_version=$manifest_version
fi

commit_range="HEAD"
if [[ -n "$latest_stable_tag" ]]; then
  commit_range="${latest_stable_tag}..HEAD"
fi

bump_level=0
breaking_header_pattern='^[[:lower:]][[:alnum:]-]*(\([^)]+\))?!:'
feat_header_pattern='^feat(\([^)]+\))?:'
while IFS= read -r -d '' commit; do
  subject=${commit%%$'\n'*}
  if [[ $subject =~ $breaking_header_pattern ]] || grep -Eq '(^|[[:space:]])BREAKING CHANGE:' <<<"$commit"; then
    bump_level=2
    break
  fi

  if [[ $subject =~ $feat_header_pattern ]]; then
    bump_level=1
  fi
done < <(git log --format='%s%n%b%x00' $commit_range)

IFS='.' read -r major minor patch <<<"$baseline_version"
case "$bump_level" in
  2)
    major=$((major + 1))
    minor=0
    patch=0
    ;;
  1)
    minor=$((minor + 1))
    patch=0
    ;;
  *)
    patch=$((patch + 1))
    ;;
esac

stable_version="${major}.${minor}.${patch}"

case "$branch_name" in
  production)
    version="$stable_version"
    tag="v${version}"
    prerelease="false"
    cargo_args="--release"
    profile_dir="release"
    ;;
  development)
    prerelease_count=$(git rev-list --count "$commit_range")
    if [[ "$prerelease_count" -lt 1 ]]; then
      prerelease_count=1
    fi

    version="${stable_version}-dev.${prerelease_count}"
    tag="v${version}"
    prerelease="true"
    cargo_args=""
    profile_dir="debug"
    ;;
  *)
    echo "Unsupported release branch: $branch_name" >&2
    exit 1
    ;;
esac

{
  echo "version=$version"
  echo "tag=$tag"
  echo "prerelease=$prerelease"
  echo "cargo_args=$cargo_args"
  echo "profile_dir=$profile_dir"
} >>"$GITHUB_OUTPUT"
