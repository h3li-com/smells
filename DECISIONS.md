# Decisions — smells

| # | Decision |
|---|---|
| 1 | Maintain the reusable Rust code-smell scanner in a separate Git repository called `smells`, outside consuming projects. Rust is the first supported language; other languages need separate validated versioned packs. |
| 2 | Provide a standalone development executable using `src/main.rs`. Projects invoke it through a hook or quality runner and supply checked-in policies; scanner code has no application production dependencies or hard-coded project identities. |
| 3 | Deterministic verdicts use fixed pattern measurements, explicit versioned policy and pinned captured inputs. Required missing checks fail. Reports include function/type values below limits and explicit coverage statuses for all 23 Refactoring.Guru smells. Exact exceptions remain pending and nonempty exception input is rejected. |
| 4 | Keep OSV, Gitleaks, application tests, coverage generation, and hook composition with each project's broader quality runner. Preserve existing hooks; the provided hook is an inactive example until implementation and staged-snapshot fixtures pass. |
| 5 | Publish this standalone repository as private `mindful-time/smells` on GitHub so consuming projects can pin and install a future tested scanner revision. Publishing specifications does not create the scanner executable; hook integration must continue to fail when a required executable is unavailable. |
| 6 | Implement source-pattern detection before automated refactoring. rust-v1 inventories 28 rules, with 17 source detectors implemented and 11 compiler/type/contract/history detectors pending. Structural matching is deterministic but is not proof of semantic design defects or of the absence of every smell. |
| 7 | The first source scope is authored_all_cfg, including tests and inactive branches. It is not the earlier proposed active-production compiler scope. Policies explicitly adopt this scope; unsupported scope is an error. Duplication uses the documented normalized 4-token multiset algorithm; function lines use lexical source spans, independently of Clippy/cargo-crap conventions. |
