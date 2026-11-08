#!/bin/sh
set -eu
root=$(git rev-parse --show-toplevel)
cd "$root"
exec "${PLEASE_EXECUTABLE:-please}" sync "$@"
