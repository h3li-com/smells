//! LCOV line coverage that replaces the zero-coverage CRAP upper bound.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Executable-line hit counts keyed by corpus-relative source path.
#[derive(Clone, Debug, Default)]
pub struct LineCoverage {
    pub sha256: String,
    pub files: BTreeMap<String, BTreeMap<usize, u64>>,
}

impl LineCoverage {
    /// Returns `(covered, instrumented)` executable lines inside the inclusive span, or
    /// `None` when the artifact records no executable line there.
    pub fn span(&self, path: &str, first: usize, last: usize) -> Option<(u64, u64)> {
        let (covered, instrumented) = self
            .files
            .get(path)?
            .range(first..=last.max(first))
            .fold((0, 0), |(covered, instrumented), (_, hits)| {
                (covered + u64::from(*hits > 0), instrumented + 1)
            });
        (instrumented > 0).then_some((covered, instrumented))
    }
}

/// Reads an LCOV artifact and keeps only records for files in the scanned corpus.
pub fn load(
    path: &Path,
    root: &Path,
    corpus: &BTreeMap<String, String>,
) -> Result<LineCoverage, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read coverage {}: {error}", path.display()))?;
    parse(&bytes, root, corpus)
}

pub fn parse(
    bytes: &[u8],
    root: &Path,
    corpus: &BTreeMap<String, String>,
) -> Result<LineCoverage, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "coverage file is not UTF-8 LCOV")?;
    let mut reader = Reader {
        roots: candidate_roots(root),
        corpus,
        files: BTreeMap::new(),
        records: 0,
        current: None,
    };
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end();
        reader
            .read(line)
            .map_err(|()| format!("malformed LCOV at line {}: {line}", number + 1))?;
    }
    let files = reader.finish(root)?;
    reject_stale(&files, corpus)?;
    Ok(LineCoverage {
        sha256: crate::input::digest(&[bytes]),
        files,
    })
}

/// LCOV record state; `current` is `Some(None)` inside a record for a file outside
/// the scanned corpus.
struct Reader<'a> {
    roots: Vec<PathBuf>,
    corpus: &'a BTreeMap<String, String>,
    files: BTreeMap<String, BTreeMap<usize, u64>>,
    records: usize,
    current: Option<Option<String>>,
}

impl Reader<'_> {
    fn read(&mut self, line: &str) -> Result<(), ()> {
        if let Some(source) = line.strip_prefix("SF:") {
            return self.open(source);
        }
        if let Some(data) = line.strip_prefix("DA:") {
            return self.data(data);
        }
        if line == "end_of_record" {
            self.current.take().ok_or(())?;
        }
        Ok(())
    }

    fn open(&mut self, source: &str) -> Result<(), ()> {
        if self.current.is_some() {
            return Err(());
        }
        self.records += 1;
        let path = corpus_path(source, &self.roots).filter(|path| self.corpus.contains_key(path));
        self.current = Some(path);
        Ok(())
    }

    fn data(&mut self, data: &str) -> Result<(), ()> {
        let record = self.current.as_ref().ok_or(())?;
        let (line_number, hits) = data_fields(data).ok_or(())?;
        if let Some(path) = record {
            let entry = self
                .files
                .entry(path.clone())
                .or_default()
                .entry(line_number)
                .or_default();
            *entry = entry.saturating_add(hits);
        }
        Ok(())
    }

    fn finish(self, root: &Path) -> Result<BTreeMap<String, BTreeMap<usize, u64>>, String> {
        if self.current.is_some() {
            return Err("coverage file ends inside an LCOV record".into());
        }
        if self.records == 0 {
            return Err("coverage file contains no LCOV records".into());
        }
        if self.files.is_empty() {
            return Err(format!(
                "coverage file matches no scanned source file under {}; regenerate it for this checkout",
                root.display()
            ));
        }
        Ok(self.files)
    }
}

/// Parses `line,hits[,checksum]`; line numbers are 1-based.
fn data_fields(data: &str) -> Option<(usize, u64)> {
    let mut fields = data.split(',');
    let line_number = fields.next()?.parse().ok().filter(|line| *line > 0)?;
    let hits = fields.next()?.parse().ok()?;
    Some((line_number, hits))
}

fn reject_stale(
    files: &BTreeMap<String, BTreeMap<usize, u64>>,
    corpus: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (path, lines) in files {
        let length = corpus[path].lines().count();
        if lines.keys().next_back().is_some_and(|last| *last > length) {
            return Err(format!(
                "coverage is stale for {path}: it records line beyond the file's {length} lines"
            ));
        }
    }
    Ok(())
}

fn candidate_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.to_path_buf()];
    if let Ok(canonical) = fs::canonicalize(root)
        && canonical != root
    {
        roots.push(canonical);
    }
    roots
}

fn corpus_path(source: &str, roots: &[PathBuf]) -> Option<String> {
    let source = Path::new(source);
    let relative = if source.is_absolute() {
        roots
            .iter()
            .find_map(|root| source.strip_prefix(root).ok())?
    } else {
        source
    };
    let text = relative.to_string_lossy().replace('\\', "/");
    Some(text.trim_start_matches("./").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> BTreeMap<String, String> {
        BTreeMap::from([("src/lib.rs".to_string(), "a\nb\nc\nd\n".to_string())])
    }

    #[test]
    fn absolute_and_relative_records_map_to_corpus_paths() {
        let root = Path::new("/repo");
        let lcov = b"TN:\nSF:/repo/src/lib.rs\nDA:1,3\nDA:2,0\nend_of_record\nSF:./src/lib.rs\nDA:2,1\nDA:4,0\nend_of_record\nSF:/elsewhere/dep.rs\nDA:9,1\nend_of_record\n";
        let coverage = parse(lcov, root, &corpus()).expect("valid LCOV");
        assert_eq!(
            coverage.files["src/lib.rs"],
            BTreeMap::from([(1, 3), (2, 1), (4, 0)])
        );
        assert_eq!(coverage.span("src/lib.rs", 1, 4), Some((2, 3)));
        assert_eq!(coverage.span("src/lib.rs", 3, 3), None);
        assert_eq!(coverage.span("src/lib.rs", 4, 2), Some((0, 1)));
        assert_eq!(coverage.span("src/other.rs", 1, 4), None);
        assert_eq!(coverage.sha256, crate::input::digest(&[lcov]));
    }

    #[test]
    fn malformed_unmatched_and_stale_coverage_fails_closed() {
        let root = Path::new("/repo");
        for (lcov, expected) in [
            (&b"DA:1,1\n"[..], "malformed LCOV at line 1"),
            (
                b"SF:src/lib.rs\nSF:src/lib.rs\n",
                "malformed LCOV at line 2",
            ),
            (b"SF:src/lib.rs\nDA:x,1\nend_of_record\n", "malformed LCOV"),
            (b"SF:src/lib.rs\nDA:1\nend_of_record\n", "malformed LCOV"),
            (b"SF:src/lib.rs\nDA:0,1\nend_of_record\n", "malformed LCOV"),
            (b"end_of_record\n", "malformed LCOV at line 1"),
            (b"SF:src/lib.rs\nDA:1,1\n", "ends inside an LCOV record"),
            (b"TN:\n", "contains no LCOV records"),
            (
                b"SF:/other/src/lib.rs\nDA:1,1\nend_of_record\n",
                "matches no scanned source",
            ),
            (
                b"SF:src/lib.rs\nDA:5,1\nend_of_record\n",
                "coverage is stale for src/lib.rs",
            ),
            (b"\xff", "not UTF-8"),
        ] {
            let error = parse(lcov, root, &corpus()).expect_err(expected);
            assert!(error.contains(expected), "{error} lacks {expected}");
        }
    }

    #[test]
    fn records_under_the_canonical_root_match_a_non_canonical_root() {
        let base =
            std::env::temp_dir().join(format!("smells-coverage-root-{}", std::process::id()));
        fs::create_dir_all(base.join("repo")).unwrap();
        let root = base.join("repo").join("..").join("repo");
        let canonical = fs::canonicalize(&root).unwrap();
        assert_ne!(canonical, root);
        let lcov = format!(
            "SF:{}\nDA:1,1\nend_of_record\n",
            canonical.join("src/lib.rs").display()
        );
        let coverage = parse(lcov.as_bytes(), &root, &corpus()).expect("canonical record matches");
        assert_eq!(coverage.span("src/lib.rs", 1, 1), Some((1, 1)));
        assert_eq!(candidate_roots(&canonical), vec![canonical.clone()]);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn exact_last_line_is_not_stale() {
        let coverage = parse(
            b"SF:src/lib.rs\nDA:4,1\nend_of_record\n",
            Path::new("/r"),
            &corpus(),
        )
        .expect("last line is in range");
        assert_eq!(coverage.span("src/lib.rs", 4, 4), Some((1, 1)));
    }
}
