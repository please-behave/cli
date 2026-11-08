#!/bin/sh
# Validate the final message before Git creates the commit. No runtime dependencies.
set -eu
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
    printf '%s\n' 'Usage: commit-msg <message-file>' >&2
    exit 1
fi
root=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
policy="$root/.githooks/commit-policy"
if [ ! -r "$policy" ]; then
    printf '%s\n' "Missing commit policy: $policy" >&2
    exit 1
fi
if ! LC_ALL=C awk '
    FNR == NR {
        if (NF != 2 || $2 !~ /^[a-z][a-z0-9-]*$/) { invalid = 1; next }
        if ($1 == "type") types = types (types == "" ? "" : "|") $2
        else if ($1 == "scope") scopes = scopes (scopes == "" ? "" : "|") $2
        else invalid = 1
        next
    }
    FNR == 1 {
        seen = 1
        sub(/\r$/, "")
        header = "^(" types ")(\\((" scopes ")\\))?!?: [^[:space:][:cntrl:]][^[:cntrl:]]*$"
        if ($0 ~ header && $0 !~ /[[:space:]]$/) exit 0
        exit 1
    }
    END { if (!seen || invalid || types == "" || scopes == "") exit 1 }
' "$policy" "$1"; then
    cat >&2 <<'MESSAGE'
Invalid commit message.
Use: type(scope): summary
MESSAGE
    awk '$1 == "type" { types = types (types == "" ? "" : ", ") $2 }
         $1 == "scope" { scopes = scopes (scopes == "" ? "" : ", ") $2 }
         END { print "Allowed types: " types; print "Optional scopes: " scopes }' "$policy" >&2
    cat >&2 <<'MESSAGE'
Use a nonempty summary without leading/trailing whitespace. Breaking-change ! is optional.
Examples: chore(deps): update dependencies | feat(reviews): add focused context selection
MESSAGE
    exit 1
fi
