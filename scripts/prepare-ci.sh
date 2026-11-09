#!/usr/bin/env bash
# Explicit preparation for the Ubuntu x64 quality job; never called by a Rule.
set -euo pipefail

npm ci --ignore-scripts
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
cargo +stable fetch --locked
rustup toolchain install 1.85.0 --profile minimal
cargo +1.85.0 fetch --locked
go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
echo "$(go env GOPATH)/bin" >> "$GITHUB_PATH"

bash scripts/prepare-release-readiness.sh
