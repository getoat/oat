#!/bin/sh

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)

docker run --rm \
    -v "$repo_root:/work" \
    -w /work \
    rust:1.88-bullseye \
    bash -lc '
set -eu
export PATH="/usr/local/cargo/bin:$PATH"
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates
cargo build --locked --bin oat --release --target-dir target/compat-gnu
mkdir -p target/compat-gnu/runtime
cp /usr/lib/x86_64-linux-gnu/libssl.so.1.1 target/compat-gnu/runtime/
cp /usr/lib/x86_64-linux-gnu/libcrypto.so.1.1 target/compat-gnu/runtime/
'
