#!/bin/sh
set -eu
root=$(git rev-parse --show-toplevel)
cd "$root"
# Retire the old repository-owned path; Hannes manages actual hook state.
current=$(git config --local --get core.hooksPath || :)
case "$current" in
    .githooks|"$root/.githooks")
        for event in pre-commit commit-msg pre-push; do
            if [ -e ".githooks/$event" ] || [ -L ".githooks/$event" ]; then
                printf '%s\n' "Existing hook preserved: .githooks/$event. Move custom content to Git's hook directory before retiring this path." >&2
                exit 1
            fi
        done
        git config --local --unset core.hooksPath
        ;;
esac
exec "${HANNES_EXECUTABLE:-hannes}" pass auf "$@"
