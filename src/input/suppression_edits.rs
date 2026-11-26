//! Finds file changes in the history window whose only edit adds, removes or
//! moves a lint or type-checker suppression. Such a change silences a tool; it
//! does not change what the file does, so history rules do not count it.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

/// Comment markers that suppress a linter, formatter, type checker or coverage
/// tool for the line or block they annotate.
const SUPPRESSION_MARKERS: &[&str] = &[
    "eslint-disable",
    "eslint-enable",
    "oxlint-disable",
    "oxlint-enable",
    "biome-ignore",
    "prettier-ignore",
    "@ts-expect-error",
    "@ts-ignore",
    "@ts-nocheck",
    "istanbul ignore",
    "c8 ignore",
    "noqa",
    "nosec",
    "type: ignore",
    "pyright: ignore",
    "pylint: disable",
    "pylint: enable",
    "pragma: no cover",
    "fmt: off",
    "fmt: on",
    "fmt: skip",
];

/// Rust attributes that only suppress lints.
const SUPPRESSION_ATTRIBUTES: &[&str] = &["#[allow(", "#[expect(", "#![allow(", "#![expect("];

/// The `(commit, path)` changes in the history window that only edit
/// suppressions. Missing objects are never fetched: in a partial clone, or when
/// Git fails, no change is discounted and every changed file still counts.
pub(super) fn suppression_only_edits(
    root: &Path,
    revision: Option<&str>,
    limit: &str,
    files: &BTreeMap<String, String>,
) -> BTreeSet<(String, String)> {
    let mut args = vec![
        "-c",
        "core.quotepath=false",
        "log",
        "--format=%x1e%H",
        "--patch",
        "--unified=0",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--relative",
        "-n",
        limit,
    ];
    if let Some(commit) = revision {
        args.push(commit);
    }
    args.extend(["--", "."]);
    let output = Command::new("git")
        .arg("--no-replace-objects")
        .args(&args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .current_dir(root)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse_patch_log(&String::from_utf8_lossy(&output.stdout), files)
        }
        _ => BTreeSet::new(),
    }
}

/// One file's changed lines in one commit.
#[derive(Default)]
struct FileEdit {
    path: Option<String>,
    hunks: bool,
    removed: Vec<String>,
    added: Vec<String>,
}

impl FileEdit {
    fn suppression_only(&self) -> bool {
        self.hunks && lines_differ_only_in_suppressions(&self.removed, &self.added)
    }
}

/// Parses `git log --patch --unified=0` output. Hunk line counts decide which
/// lines are content, so changed lines that look like headers are read as
/// content.
fn parse_patch_log(log: &str, files: &BTreeMap<String, String>) -> BTreeSet<(String, String)> {
    let mut found = BTreeSet::new();
    let mut commit = String::new();
    let mut edit = FileEdit::default();
    let mut remaining = 0_usize;
    let mut finish = |commit: &str, edit: &mut FileEdit| {
        let done = std::mem::take(edit);
        if let Some(path) = &done.path
            && files.contains_key(path)
            && done.suppression_only()
        {
            found.insert((commit.to_string(), path.clone()));
        }
    };
    for line in log.split('\n') {
        if remaining > 0 {
            if line.starts_with('\\') {
                continue;
            }
            remaining -= 1;
            if let Some(added) = line.strip_prefix('+') {
                edit.added.push(added.to_string());
            } else if let Some(removed) = line.strip_prefix('-') {
                edit.removed.push(removed.to_string());
            }
        } else if let Some(header) = line.strip_prefix('\u{1e}') {
            finish(&commit, &mut edit);
            commit = header.trim().to_string();
        } else if let Some(paths) = line.strip_prefix("diff --git ") {
            finish(&commit, &mut edit);
            edit.path = diff_path(paths);
        } else if let Some(range) = line.strip_prefix("@@ ") {
            edit.hunks = true;
            remaining = hunk_lines(range);
        }
    }
    finish(&commit, &mut edit);
    found
}

/// The path in a `diff --git a/P b/P` header. Renames are disabled, so both
/// sides name the same path and the header splits at its midpoint.
fn diff_path(paths: &str) -> Option<String> {
    let half = paths.len().checked_sub(1)? / 2;
    let (old, new) = (paths.get(..half)?, paths.get(half + 1..)?);
    let path = old.strip_prefix("a/")?;
    (new.strip_prefix("b/")? == path).then(|| path.to_string())
}

/// The number of content lines in a `-a,b +c,d @@` hunk header.
fn hunk_lines(range: &str) -> usize {
    range
        .split_whitespace()
        .take(2)
        .filter_map(|side| {
            let counts = side.get(1..)?;
            Some(match counts.split_once(',') {
                Some((_, count)) => count.parse().unwrap_or(0),
                None => 1,
            })
        })
        .sum()
}

/// Whether the removed and added lines are the same code once suppression
/// comments and attributes are taken out, and at least one line carried one.
fn lines_differ_only_in_suppressions(removed: &[String], added: &[String]) -> bool {
    let mut suppressed = false;
    let mut code = |lines: &[String]| {
        let mut kept = lines
            .iter()
            .filter_map(|line| {
                let stripped = without_suppression(line);
                suppressed |= stripped.len() != line.trim_end().len();
                (!stripped.trim().is_empty() || line.trim().is_empty())
                    .then(|| stripped.to_string())
            })
            .collect::<Vec<_>>();
        kept.sort();
        kept
    };
    let (removed, added) = (code(removed), code(added));
    suppressed && removed == added
}

/// The line without a trailing suppression comment, or empty when the line is
/// only a suppression comment or attribute.
fn without_suppression(line: &str) -> &str {
    let line = line.trim_end();
    let trimmed = line.trim_start();
    if SUPPRESSION_ATTRIBUTES
        .iter()
        .any(|attribute| trimmed.starts_with(attribute))
        && trimmed.ends_with(']')
    {
        return "";
    }
    let lower = line.to_ascii_lowercase();
    let Some(marker) = SUPPRESSION_MARKERS
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
    else {
        return line;
    };
    let opener = ["//", "/*", "#"]
        .iter()
        .filter_map(|opener| line[..marker].rfind(opener))
        .max();
    match opener {
        Some(start) => line[..start].trim_end(),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::{diff_path, hunk_lines, lines_differ_only_in_suppressions, parse_patch_log};
    use std::collections::BTreeMap;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn diff_headers_name_the_path_on_both_sides() {
        assert_eq!(
            diff_path("a/src/x.ts b/src/x.ts").as_deref(),
            Some("src/x.ts")
        );
        assert_eq!(diff_path("a/a b/c b/a b/c").as_deref(), Some("a b/c"));
        assert_eq!(diff_path("a/x.ts b/y.ts"), None);
        assert_eq!(diff_path(""), None);
    }

    #[test]
    fn hunk_headers_count_both_sides_with_an_implicit_one() {
        assert_eq!(hunk_lines("-3,2 +4,5 @@ fn x()"), 7);
        assert_eq!(hunk_lines("-3 +4 @@"), 2);
        assert_eq!(hunk_lines("-3,0 +4 @@"), 1);
    }

    #[test]
    fn suppression_edits_differ_only_in_suppression_comments() {
        assert!(lines_differ_only_in_suppressions(
            &[],
            &lines(&["  // eslint-disable-next-line @typescript-eslint/no-unsafe-enum-assignment"]),
        ));
        assert!(lines_differ_only_in_suppressions(
            &lines(&["  let flags: ts.TypeFlags = 0;"]),
            &lines(&["  let flags: ts.TypeFlags = 0; // eslint-disable-line no-x"]),
        ));
        assert!(lines_differ_only_in_suppressions(
            &lines(&["import os  # noqa: F401"]),
            &lines(&["import os"]),
        ));
        assert!(lines_differ_only_in_suppressions(
            &[],
            &lines(&["    #[expect(clippy::too_many_lines, reason = \"table\")]"]),
        ));
        assert!(lines_differ_only_in_suppressions(
            &lines(&["x = f()  # type: ignore[attr]"]),
            &lines(&["x = f()  # pyright: ignore[reportX]"]),
        ));
    }

    #[test]
    fn code_changes_and_moves_are_not_suppression_edits() {
        assert!(!lines_differ_only_in_suppressions(
            &lines(&["  let flags = 0;"]),
            &lines(&["  let flags = 1; // eslint-disable-line no-x"]),
        ));
        assert!(!lines_differ_only_in_suppressions(
            &lines(&["a();", "b();"]),
            &lines(&["b();", "a();"]),
        ));
        assert!(!lines_differ_only_in_suppressions(
            &[],
            &lines(&["// eslint-disable-next-line no-x", "run();"]),
        ));
        assert!(!lines_differ_only_in_suppressions(
            &[],
            &lines(&["#[allow(dead_code)] fn unused() {}"]),
        ));
        assert!(!lines_differ_only_in_suppressions(&[], &[]));
    }

    #[test]
    fn patch_logs_report_suppression_only_files_per_commit() {
        let files = BTreeMap::from([
            ("a.ts".to_string(), String::new()),
            ("b.ts".to_string(), String::new()),
            ("c.ts".to_string(), String::new()),
        ]);
        let log = "\u{1e}one\n\ndiff --git a/a.ts b/a.ts\nindex 1..2 100644\n--- a/a.ts\n+++ b/a.ts\n\
@@ -3,0 +4 @@ x\n+// eslint-disable-next-line no-x\n\
diff --git a/b.ts b/b.ts\n--- a/b.ts\n+++ b/b.ts\n@@ -1 +1,2 @@\n--- not a header\n\
+++ not a header // eslint-disable-line\n+run();\n\\ No newline at end of file\n\
diff --git a/untracked.ts b/untracked.ts\n@@ -0,0 +1 @@\n+// @ts-ignore\n\
diff --git a/c.ts b/c.ts\nnew file mode 100644\n\
\u{1e}two\n\ndiff --git a/b.ts b/b.ts\n@@ -2 +2 @@\n-x = 1;\n+x = 1; // @ts-expect-error why\n";
        let found = parse_patch_log(log, &files);
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            vec![
                ("one".to_string(), "a.ts".to_string()),
                ("two".to_string(), "b.ts".to_string()),
            ]
        );
    }
}
