#!/bin/sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

configured=$(git config --local --get core.hooksPath || true)
if [ -n "$configured" ] && [ "$configured" != ".githooks" ]; then
    printf 'install-hooks: refusing to replace existing core.hooksPath=%s\n' "$configured" >&2
    exit 2
fi

if [ -z "$configured" ]; then
    for hook in pre-commit pre-push; do
        if [ -e ".git/hooks/$hook" ]; then
            printf 'install-hooks: refusing to bypass existing .git/hooks/%s\n' "$hook" >&2
            exit 2
        fi
    done
fi

git config --local core.hooksPath .githooks
printf 'installed repository hooks through core.hooksPath=.githooks\n'
