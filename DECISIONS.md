# Decisions — smells

| # | Decision |
|---|---|
| 1 | Maintain the reusable Rust code-smell scanner in a separate Git repository called `smells`, outside consuming projects. The current repository holds specifications; implementation remains pending. |
| 2 | Provide a standalone development executable using `src/main.rs`. Projects invoke it through a hook or quality runner and supply checked-in policies; scanner code has no application production dependencies or hard-coded project identities. |
| 3 | Deterministic verdicts use fixed pattern measurements, recorded thresholds, exact exceptions, and pinned inputs. Required missing checks fail. Reports include measured values below limits and explicit coverage statuses for all 23 Refactoring.Guru smells. |
| 4 | Keep OSV, Gitleaks, application tests, coverage generation, and hook composition with each project's broader quality runner. Preserve existing hooks; the provided hook is an inactive example until implementation and staged-snapshot fixtures pass. |
| 5 | Publish this standalone repository as private `mindful-time/smells` on GitHub so consuming projects can pin and install a future tested scanner revision. Publishing specifications does not create the scanner executable; hook integration must continue to fail when a required executable is unavailable. |
