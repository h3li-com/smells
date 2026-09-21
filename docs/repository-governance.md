# Repository governance

## Intended `main` policy

`main` is owner-maintained and pull-request-only:

- `mindful-time` is the only account with write or administration permission.
- Outside contributors work from forks and open pull requests.
- Required CI rejects a pull request when its head branch is in this repository
  instead of a fork, including a same-repository pull request opened by the owner.
  Historical PR #1 has an exact owner-and-branch-bound bootstrap exception because
  it introduced this rule before branch protection was enabled. A merged pull
  request cannot be reopened, and no later pull request can match its number.
- Direct pushes, force pushes, and branch deletion are blocked for everyone,
  including the owner.
- The branch must be current with `main`, have resolved conversations, use linear
  history, and pass the `required` CI status before merge.
- Zero approving reviews are required because GitHub does not allow an author to
  approve their own pull request. The owner remains the only account able to merge.

`.github/CODEOWNERS` records ownership. Enforcement comes from branch protection,
not from the CODEOWNERS file or local Git hooks.

## Enforced GitHub settings

The repository is public so GitHub can enforce this policy without requiring a
paid private-repository plan. It still has exactly one collaborator with write
access: `mindful-time`. Apply or audit the checked-in branch-protection rule with:

```sh
./scripts/configure-main-protection.sh mindful-time/smells
```

Then inspect the resulting settings in GitHub and confirm a direct owner push is
rejected. Also confirm that a same-repository pull request fails the fork-origin CI
check. GitHub branch protection has no native fork-origin setting; the required CI
check supplies that policy. The script enables administrator enforcement and does
not configure an owner bypass.

The release and registry-publisher workflows independently reject dispatches and
reruns from every actor other than `mindful-time`, even if another writer is added
later. Every publication job also requires approval through the protected `release`
environment. Immutable releases lock each published release's tag and assets.
