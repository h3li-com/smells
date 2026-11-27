//! Finds file changes in the history window that leave the code as it was: the
//! only edit adds, removes or moves a lint or type-checker suppression, only
//! changes a version string, or moves the file unchanged from another path.
//! Such a change silences a tool, marks a release or renames a file; it does
//! not change what the file does, so history rules do not count it.

use crate::suppression_markers::SUPPRESSION_MARKERS;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};

/// Rust attributes that only suppress lints.
const SUPPRESSION_ATTRIBUTES: &[&str] = &["#[allow(", "#[expect(", "#![allow(", "#![expect("];

/// The `(commit, path)` changes in the history window that only edit
/// suppressions, a version string, or move the file unchanged. Missing objects are never fetched: in a partial clone, or when
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
        "-c",
        "diff.interHunkContext=0",
        "log",
        "--format=%x1e%H",
        "--patch",
        "--unified=0",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--relative",
        "-n",
        limit,
    ];
    if let Some(commit) = revision {
        args.push(commit);
    }
    args.extend(["--", "."]);
    let child = Command::new("git")
        .arg("--no-replace-objects")
        .args(&args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return BTreeSet::new();
    };
    // Streamed, so memory holds one commit's patch rather than the whole window.
    let found = child
        .stdout
        .take()
        .map(|stdout| parse_patch_log(BufReader::new(stdout), files));
    match (found, child.wait()) {
        (Some(Ok(found)), Ok(status)) if status.success() => found,
        _ => BTreeSet::new(),
    }
}

/// One file's changed lines in one commit.
#[derive(Default)]
struct FileEdit {
    path: Option<String>,
    hunks: bool,
    created: bool,
    deleted: bool,
    removed: Vec<String>,
    added: Vec<String>,
}

impl FileEdit {
    fn suppression_only(&self) -> bool {
        self.hunks && lines_differ_only_in_suppressions(&self.removed, &self.added)
    }

    /// Whether every changed line assigns a version string, a release bump.
    fn version_only(&self) -> bool {
        !self.added.is_empty()
            && self.removed.len() == self.added.len()
            && self
                .removed
                .iter()
                .chain(&self.added)
                .all(|line| version_assignment(line))
    }
}

/// Whether a line assigns a string literal to a variable or constant named
/// `version`, `VERSION` or `__version__`.
fn version_assignment(line: &str) -> bool {
    let Some((target, value)) = line.split_once('=') else {
        return false;
    };
    let name = target.split(':').next().unwrap_or_default().trim();
    let name = name.rsplit(' ').next().unwrap_or_default();
    let value = value.trim().trim_end_matches([';', ',']).trim_end();
    matches!(name, "version" | "VERSION" | "__version__")
        && value.len() >= 2
        && ['"', '\''].iter().any(|quote| {
            value.starts_with(*quote)
                && value.ends_with(*quote)
                && !value[1..value.len() - 1].contains(*quote)
        })
}

/// Parses `git log --patch --unified=0` output. Hunk line counts decide which
/// lines are content, so changed lines that look like headers are read as
/// content. Each side is counted separately: a context line uses one line of
/// both.
/// The lines of the current hunk still to read on the old and new side.
#[derive(Default)]
struct HunkLines {
    old: usize,
    new: usize,
}

impl HunkLines {
    fn open(&self) -> bool {
        self.old > 0 || self.new > 0
    }

    /// Records one hunk line in `edit`. A context line uses up both sides, and a
    /// `\ No newline at end of file` marker uses up neither.
    fn read(&mut self, line: &str, edit: &mut FileEdit) {
        if let Some(added) = line.strip_prefix('+') {
            self.new = self.new.saturating_sub(1);
            edit.added.push(added.to_string());
        } else if let Some(removed) = line.strip_prefix('-') {
            self.old = self.old.saturating_sub(1);
            edit.removed.push(removed.to_string());
        } else if !line.starts_with('\\') {
            self.old = self.old.saturating_sub(1);
            self.new = self.new.saturating_sub(1);
        }
    }
}

impl FileEdit {
    /// Records a file header line, returning the hunk a `@@` line opens.
    fn read_header(&mut self, line: &str) -> Option<HunkLines> {
        if line.starts_with("new file mode") {
            self.created = true;
        } else if line.starts_with("deleted file mode") {
            self.deleted = true;
        } else if let Some(range) = line.strip_prefix("@@ ") {
            self.hunks = true;
            let (old, new) = hunk_lines(range);
            return Some(HunkLines { old, new });
        }
        None
    }
}

fn parse_patch_log(
    log: impl BufRead,
    files: &BTreeMap<String, String>,
) -> io::Result<BTreeSet<(String, String)>> {
    let mut found = BTreeSet::new();
    let mut commit = String::new();
    let mut edits = Vec::new();
    let mut edit = FileEdit::default();
    let mut hunk = HunkLines::default();
    for line in log.split(b'\n') {
        let line = String::from_utf8_lossy(&line?).into_owned();
        let line = line.as_str();
        if hunk.open() {
            hunk.read(line, &mut edit);
        } else if let Some(header) = line.strip_prefix('\u{1e}') {
            edits.push(std::mem::take(&mut edit));
            found.extend(unchanged_files(&commit, &std::mem::take(&mut edits), files));
            commit = header.trim().to_string();
        } else if let Some(paths) = line.strip_prefix("diff --git ") {
            edits.push(std::mem::take(&mut edit));
            edit.path = diff_path(paths);
        } else if let Some(opened) = edit.read_header(line) {
            hunk = opened;
        }
    }
    edits.push(edit);
    found.extend(unchanged_files(&commit, &edits, files));
    Ok(found)
}

/// The `(commit, path)` pairs of one commit whose edit leaves the code as it
/// was: it only edits suppressions, only changes a version string, or creates
/// the file with exactly the lines the commit deletes from another file, a move.
fn unchanged_files(
    commit: &str,
    edits: &[FileEdit],
    files: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let moved_from = |edit: &FileEdit| {
        edit.created
            && !edit.added.is_empty()
            && edits
                .iter()
                .any(|other| other.deleted && other.removed == edit.added)
    };
    edits
        .iter()
        .filter(|edit| edit.suppression_only() || edit.version_only() || moved_from(edit))
        .filter_map(|edit| edit.path.clone())
        .filter(|path| files.contains_key(path))
        .map(|path| (commit.to_string(), path))
        .collect()
}

/// The path in a `diff --git a/P b/P` header. Renames are disabled, so both
/// sides name the same path and the header splits at its midpoint.
fn diff_path(paths: &str) -> Option<String> {
    let half = paths.len().checked_sub(1)? / 2;
    let (old, new) = (paths.get(..half)?, paths.get(half + 1..)?);
    let path = old.strip_prefix("a/")?;
    (new.strip_prefix("b/")? == path).then(|| path.to_string())
}

/// The old and new line counts in a `-a,b +c,d @@` hunk header.
fn hunk_lines(range: &str) -> (usize, usize) {
    let mut sides = range.split_whitespace().take(2).map(|side| {
        side.get(1..)
            .map_or(0, |counts| match counts.split_once(',') {
                Some((_, count)) => count.parse().unwrap_or(0),
                None => 1,
            })
    });
    (sides.next().unwrap_or(0), sides.next().unwrap_or(0))
}

/// Whether the removed and added lines are the same code once suppression
/// comments and attributes are taken out, and at least one line carried one.
fn lines_differ_only_in_suppressions(removed: &[String], added: &[String]) -> bool {
    let mut suppressed = false;
    let mut code = |lines: &[String]| {
        lines
            .iter()
            .filter_map(|line| {
                let stripped = without_suppression(line);
                suppressed |= stripped.len() != line.trim_end().len();
                (!stripped.trim().is_empty() || line.trim().is_empty())
                    .then(|| stripped.to_string())
            })
            .collect::<Vec<_>>()
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
    use super::{
        diff_path, hunk_lines, lines_differ_only_in_suppressions, parse_patch_log,
        version_assignment,
    };
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
        assert_eq!(hunk_lines("-3,2 +4,5 @@ fn x()"), (2, 5));
        assert_eq!(hunk_lines("-3 +4 @@"), (1, 1));
        assert_eq!(hunk_lines("-3,0 +4 @@"), (0, 1));
        assert_eq!(hunk_lines(""), (0, 0));
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
            &lines(&["a();", "b();"]),
            &lines(&["b(); // eslint-disable-line", "a();"]),
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
    fn version_assignments_set_a_version_name_to_a_string() {
        for line in [
            "__version__ = \"0.28.0\"",
            "export const version = '4.1.0';",
            "pub const VERSION: &str = \"1.2.3\";",
            "  version=\"2\",",
        ] {
            assert!(version_assignment(line), "{line}");
        }
        for line in [
            "__version__ = get_version()",
            "minimum_version = \"3.9\"",
            "version = \"a\" + \"b\"",
            "version = \"",
            "version == \"1\"",
            "__title__ = \"httpx\"",
        ] {
            assert!(!version_assignment(line), "{line}");
        }
    }

    #[test]
    fn context_lines_and_moves_into_existing_files_keep_commits_apart() {
        let files = BTreeMap::from([
            ("a.ts".to_string(), String::new()),
            ("b.ts".to_string(), String::new()),
        ]);
        // A merged hunk with a context line, then a commit that only adds an ignore.
        let log = "\u{1e}one\n\ndiff --git a/a.ts b/a.ts\n@@ -1,3 +1,3 @@\n-a\n+A\n b\n-c\n+C\n\
\u{1e}two\n\ndiff --git a/b.ts b/b.ts\n@@ -3,0 +4 @@\n+// eslint-disable-next-line no-x\n\
\u{1e}three\n\ndiff --git a/a.ts b/a.ts\n@@ -1,2 +0,0 @@\n-x = 1;\n-y = 2;\n\
diff --git a/b.ts b/b.ts\n@@ -9,0 +10,2 @@\n+x = 1;\n+y = 2;\n";
        assert_eq!(
            parse_patch_log(log.as_bytes(), &files)
                .expect("an in-memory log reads")
                .into_iter()
                .collect::<Vec<_>>(),
            vec![("two".to_string(), "b.ts".to_string())]
        );
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
\u{1e}two\n\ndiff --git a/b.ts b/b.ts\n@@ -2 +2 @@\n-x = 1;\n+x = 1; // @ts-expect-error why\n\
\u{1e}three\n\ndiff --git a/old/a.ts b/old/a.ts\ndeleted file mode 100644\n@@ -1,2 +0,0 @@\n-x = 1;\n-y = 2;\n\
diff --git a/a.ts b/a.ts\nnew file mode 100644\n@@ -0,0 +1,2 @@\n+x = 1;\n+y = 2;\n\
diff --git a/c.ts b/c.ts\nnew file mode 100644\n@@ -0,0 +1 @@\n+x = 1;\n\
\u{1e}four\n\ndiff --git a/b.ts b/b.ts\n@@ -3 +3 @@\n-export const version = '1.0';\n+export const version = '1.1';\n\
diff --git a/c.ts b/c.ts\n@@ -1 +1 @@\n-x = 1;\n+x = 2;\n";
        let found = parse_patch_log(log.as_bytes(), &files).expect("an in-memory log reads");
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            vec![
                ("four".to_string(), "b.ts".to_string()),
                ("one".to_string(), "a.ts".to_string()),
                ("three".to_string(), "a.ts".to_string()),
                ("two".to_string(), "b.ts".to_string()),
            ]
        );
    }
}
