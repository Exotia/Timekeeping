#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
nix develop -c cargo fmt --check
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo test
