#!/bin/sh
# Print the patch notes of a version: its section of CHANGELOG.md, without
# the heading. Fails when there is none, or when it is still marked as not
# released, so that no release goes out without notes.
#   dist/release/notes.sh 0.4.0
set -eu
cd "$(dirname "$0")/../.."
version=${1#v}
notes=$(awk -v want="## $version" '
    $0 == want { found = 1; next }
    found && /^## / { exit }
    found { print }
' CHANGELOG.md | sed -e '/./,$!d')
if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
    echo "CHANGELOG.md has no notes for $version: write them before releasing" >&2
    exit 1
fi
if printf '%s\n' "$notes" | grep -q '^\*Not released yet\.\*$'; then
    echo "the notes for $version are still marked \"Not released yet\": remove that line when they are final" >&2
    exit 1
fi
printf '%s\n' "$notes"
