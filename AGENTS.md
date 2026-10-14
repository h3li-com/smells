# Repository instructions

Run `make pre-commit-push` before committing or pushing repository changes. Preserve
unrelated working-tree changes and keep user-facing behavior fail-closed.

## Release contract

Every release must include a checked-in `docs/releases/vMAJOR.MINOR.PATCH.md` copied
from `docs/releases/TEMPLATE.md`. Do not rely on GitHub-generated release notes.

The release note must contain clear, non-empty sections in this order:

1. Why this release
2. What was added
3. What was fixed
4. What was removed
5. Before upgrading
6. How to use it
7. Verification

If nothing was removed, say that explicitly. Always state compatibility changes,
known analysis boundaries, exact versioned install commands for PyPI, npm, and
crates.io, the matching starter-policy update, and a runnable scan example. Release
claims must match `CHANGELOG.md` and verified test evidence.

`scripts/verify-release-notes.sh` is a mandatory local and workflow gate. The Release
workflow must publish the checked-in note with `--notes-file`; never replace it with
`--generate-notes`. A release is complete only when the immutable GitHub Release and
all PyPI, npmjs.com, crates.io, and GitHub Packages publications are verified.
