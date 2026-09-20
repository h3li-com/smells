# Repository governance

## Intended `main` policy

`main` is owner-maintained and pull-request-only:

- `mindful-time` is the only account with write or administration permission.
- Outside contributors work from forks and open pull requests.
- Required CI rejects a pull request when its head branch is in this repository
  instead of a fork, including a same-repository pull request opened by the owner.
- Direct pushes, force pushes, and branch deletion are blocked for everyone,
  including the owner.
- The branch must be current with `main`, have resolved conversations, use linear
  history, and pass the `required` CI status before merge.
- Zero approving reviews are required because GitHub does not allow an author to
  approve their own pull request. The owner remains the only account able to merge.

`.github/CODEOWNERS` records ownership. Enforcement comes from branch protection,
not from the CODEOWNERS file or local Git hooks.

## Current GitHub plan limitation

On 2026-09-20, GitHub returned HTTP 403 for both repository rulesets and classic
branch protection because `mindful-time/smells` is private on a plan that does not
include protected branches for private repositories. The repository currently has
exactly one collaborator with write access: `mindful-time`.

Do not make the repository public merely to work around this. Either upgrade the
account to GitHub Pro or deliberately choose public visibility. Once protection is
available and the `required` check has run at least once, apply the checked-in rule:

```sh
./scripts/configure-main-protection.sh mindful-time/smells
```

Then inspect the resulting settings in GitHub and confirm a direct owner push is
rejected. Also confirm that a same-repository pull request fails the fork-origin CI
check. GitHub branch protection has no native fork-origin setting; the required CI
check supplies that policy. The script enables administrator enforcement and does
not configure an owner bypass.
