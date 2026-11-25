use super::common::{location_at, observation};
use crate::{core::model::Input, evidence::Observation};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Assigns each changed path to the module that owns it: the first path
/// component below its implementation root, after a conventional `src`/`lib`
/// directory and a top-level Python package. Without this every file in a
/// single-package repository would share one owner.
struct Owners<'a> {
    roots: Vec<&'a str>,
    files: &'a BTreeMap<String, String>,
}

impl<'a> Owners<'a> {
    fn new(input: &'a Input) -> Self {
        Self {
            roots: input
                .implementations
                .iter()
                .filter_map(|implementation| implementation.root.as_deref())
                .filter(|root| *root != ".")
                .collect(),
            files: &input.files,
        }
    }

    fn of(&self, path: &str) -> String {
        let root = self
            .roots
            .iter()
            .filter(|root| Path::new(path).starts_with(root))
            .max_by_key(|root| Path::new(root).components().count());
        let mut prefix = root.map_or_else(PathBuf::new, PathBuf::from);
        let mut rest = Path::new(path)
            .strip_prefix(&prefix)
            .unwrap_or(Path::new(path))
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        if rest.len() > 1 && matches!(rest[0], "src" | "lib") {
            prefix.push(rest.remove(0));
        }
        if rest.len() > 1 && self.python_package(&prefix.join(rest[0])) {
            rest.remove(0);
        }
        match rest.as_slice() {
            [] => "unowned".into(),
            [file] => Path::new(file)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(file)
                .into(),
            [owner, ..] => (*owner).into(),
        }
    }

    fn python_package(&self, directory: &Path) -> bool {
        directory
            .join("__init__.py")
            .to_str()
            .is_some_and(|marker| self.files.contains_key(marker))
    }
}

/// Each changed path's owner, resolved once per scan so commit-sized loops
/// only look owners up.
struct Ownership<'a> {
    by_path: BTreeMap<&'a str, String>,
}

impl<'a> Ownership<'a> {
    fn new(input: &'a Input) -> Self {
        let owners = Owners::new(input);
        Self {
            by_path: input
                .history
                .commits
                .iter()
                .flat_map(|commit| &commit.files)
                .map(|path| (path.as_str(), owners.of(path)))
                .collect(),
        }
    }

    fn of(&self, path: &str) -> &str {
        self.by_path.get(path).map_or("unowned", String::as_str)
    }
}

pub(super) fn divergent_change(input: &Input) -> Vec<Observation> {
    if !input.history.available {
        return Vec::new();
    }
    let ownership = Ownership::new(input);
    input
        .files
        .keys()
        .filter_map(|path| {
            let touching = input
                .history
                .commits
                .iter()
                .filter(|commit| !commit.root && commit.files.contains(path))
                .collect::<Vec<_>>();
            if touching.is_empty() {
                return None;
            }
            let responsibilities = touching
                .iter()
                .flat_map(|commit| commit.files.iter())
                .filter(|changed| *changed != path)
                .map(|changed| ownership.of(changed))
                .collect::<BTreeSet<_>>();
            let source = &input.files[path];
            Some(observation(
                format!("{path}::git-change-profile"),
                location_at(path, source, 0),
                [
                    ("changes", touching.len() as u64),
                    ("responsibilities", responsibilities.len() as u64),
                ],
                json!({
                    "collector": "git_cochange_responsibilities_v1",
                    "history_limit": 200,
                    "commit_ids": touching.iter().map(|commit| &commit.id).collect::<Vec<_>>(),
                    "responsibilities": responsibilities,
                }),
            ))
        })
        .collect()
}

pub(super) fn shotgun_surgery(input: &Input) -> Vec<Observation> {
    if !input.history.available {
        return Vec::new();
    }
    let ownership = Ownership::new(input);
    input
        .history
        .commits
        .iter()
        .filter(|commit| !commit.root)
        .filter_map(|commit| {
            let owners = commit
                .files
                .iter()
                .map(|path| ownership.of(path))
                .collect::<BTreeSet<_>>();
            let anchor = commit.files.first()?;
            let source = input.files.get(anchor)?;
            Some(observation(
                format!("git:{}", commit.id),
                location_at(anchor, source, 0),
                [("owners", owners.len() as u64)],
                json!({
                    "collector": "git_commit_owner_fanout_v1",
                    "history_limit": 200,
                    "commit_id": commit.id,
                    "owners": owners,
                    "changed_files": commit.files,
                }),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Owners, Ownership, divergent_change};
    use crate::core::model::{History, HistoryCommit, Input};
    use std::collections::BTreeMap;

    fn owners<'a>(roots: &[&'a str], files: &'a BTreeMap<String, String>) -> Owners<'a> {
        Owners {
            roots: roots.to_vec(),
            files,
        }
    }

    #[test]
    fn owner_is_the_first_module_below_the_implementation_and_source_roots() {
        let none = BTreeMap::new();
        let plain = owners(&[], &none);
        assert_eq!(plain.of("backend/src/domain.rs"), "backend");
        assert_eq!(plain.of("tools/build.py"), "tools");
        assert_eq!(plain.of("Cargo.toml"), "Cargo");
        assert_eq!(plain.of("README"), "README");
        assert_eq!(plain.of("src/billing/invoice.ts"), "billing");
        assert_eq!(plain.of("src/utils.ts"), "utils");
        assert_eq!(plain.of("lib/notify/mail.rb"), "notify");
        assert_eq!(plain.of("src/main.rs"), "main");
        assert_eq!(plain.of("src"), "src");

        let rooted = owners(&["backend", "packages/web/app", "packages/web"], &none);
        assert_eq!(rooted.of("backend/src/domain/order.rs"), "domain");
        assert_eq!(rooted.of("backend/src/lib.rs"), "lib");
        assert_eq!(rooted.of("backend/Cargo.toml"), "Cargo");
        assert_eq!(rooted.of("packages/web/src/routes/page.tsx"), "routes");
        assert_eq!(rooted.of("packages/web/app/ui/button.tsx"), "ui");
        assert_eq!(rooted.of("frontend/app.ts"), "frontend");
        assert_eq!(rooted.of("backend"), "unowned");

        let packages = BTreeMap::from([
            ("shopkit/__init__.py".to_string(), String::new()),
            ("src/layout/__init__.py".to_string(), String::new()),
        ]);
        let python = owners(&[], &packages);
        assert_eq!(python.of("shopkit/billing/tax.py"), "billing");
        assert_eq!(python.of("shopkit/utils.py"), "utils");
        assert_eq!(python.of("src/layout/orders/manager.py"), "orders");
        assert_eq!(python.of("tests/test_tax.py"), "tests");
        assert_eq!(python.of("shopkit"), "shopkit");
    }

    #[test]
    fn ownership_resolves_every_changed_path_once() {
        let mut input = Input::fixture("rust-v1", BTreeMap::new());
        input.history.commits = vec![HistoryCommit {
            id: "change".into(),
            root: false,
            files: vec!["src/billing/tax.rs".into(), "src/main.rs".into()],
        }];
        let ownership = Ownership::new(&input);
        assert_eq!(ownership.of("src/billing/tax.rs"), "billing");
        assert_eq!(ownership.of("src/main.rs"), "main");
        assert_eq!(ownership.of("src/never/changed.rs"), "unowned");
    }

    #[test]
    fn divergent_change_profiles_only_the_changes_that_touch_the_file() {
        let commit = |id: &str, root: bool, files: &[&str]| HistoryCommit {
            id: id.into(),
            root,
            files: files.iter().map(|file| (*file).into()).collect(),
        };
        let mut input = Input::fixture(
            "rust-v1",
            BTreeMap::from([("src/a.rs".into(), String::new())]),
        );
        input.history = History {
            available: true,
            shallow: false,
            commits: vec![
                commit("touch", false, &["src/a.rs", "src/b/x.rs"]),
                commit("other", false, &["src/c/y.rs", "src/d/z.rs"]),
                commit("import", true, &["src/e/w.rs"]),
            ],
        };
        let observations = divergent_change(&input);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["changes"], 1);
        assert_eq!(observations[0].measurements["responsibilities"], 1);
    }
}
