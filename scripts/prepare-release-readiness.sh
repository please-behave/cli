#!/usr/bin/env bash
# Explicit preparation for CI dependency audits and security checks; never called by a Rule.
set -euo pipefail

rustup toolchain install stable --profile minimal
cargo +stable install cargo-audit --version 0.22.2 --locked

curl --fail --silent --show-error --location \
  https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_linux_x64.tar.gz \
  --output "$RUNNER_TEMP/gitleaks.tar.gz"
echo "551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb  $RUNNER_TEMP/gitleaks.tar.gz" | sha256sum --check
mkdir -p "$RUNNER_TEMP/gitleaks-bin"
tar -xzf "$RUNNER_TEMP/gitleaks.tar.gz" -C "$RUNNER_TEMP/gitleaks-bin" gitleaks
echo "$RUNNER_TEMP/gitleaks-bin" >> "$GITHUB_PATH"
