use crate::{
    core::model::{CapturedInput, History, HistoryCommit, Implementation, Input},
    policy::{Policy, Registry, SelectionOptions},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

mod suppression_edits;

use suppression_edits::suppression_only_edits;

/// Maximum number of commits captured for history-dependent rules.
pub(crate) const HISTORY_WINDOW: usize = 200;

type CapturedMigrationArtifacts = (BTreeMap<String, Vec<u8>>, BTreeMap<String, Vec<u8>>);

pub(crate) fn digest(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    format!("{:x}", hash.finalize())
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("--no-replace-objects")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run Git: {e}"))?;
    if !output.status.success() {
        return Err(format!("Git {} failed", args.first().unwrap_or(&"command")));
    }
    Ok(output.stdout)
}

fn text(bytes: Vec<u8>, label: &str) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| format!("non-UTF-8 input: {label}"))
}

fn excluded(path: &Path, policy: &Policy) -> bool {
    path.components().any(|c| matches!(c,Component::Normal(s) if policy.exclude_directories.iter().any(|e|s == e.as_str())))
}

fn source_file(path: &Path, language: &str) -> bool {
    let extension = path.extension().and_then(|value| value.to_str());
    match language {
        "rust" => extension == Some("rs"),
        "python" => matches!(extension, Some("py" | "pyi")),
        "typescript" => matches!(extension, Some("ts" | "tsx" | "mts" | "cts")),
        _ => false,
    }
}

fn migration_artifact_candidate(path: &Path, registry: &Registry) -> bool {
    path.to_str().is_some_and(|path| {
        crate::migration::migration_artifact_path_candidate(&registry.rule_pack, path)
    })
}

fn runtime_manifest(path: &Path) -> Option<(&'static str, &'static str)> {
    match path.file_name().and_then(|value| value.to_str()) {
        Some("Cargo.toml") => Some(("rust_cargo_project", "rust")),
        Some("sqlx.toml") => Some(("sqlx_configuration", "rust")),
        Some("pyproject.toml") => Some(("python_project", "python")),
        Some("package.json") => Some(("javascript_typescript_package", "javascript_typescript")),
        _ => None,
    }
}

fn migration_command_manifest(path: &Path, registry: &Registry) -> bool {
    registry.rule_pack == "rust-v1"
        && path.components().count() == 1
        && matches!(
            path.file_name().and_then(|value| value.to_str()),
            Some("Justfile" | "justfile")
        )
}

fn captured_manifest(path: &Path, registry: &Registry) -> bool {
    runtime_manifest(path).is_some() || migration_command_manifest(path, registry)
}

fn establishes_implementation_ownership(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some("Cargo.toml" | "pyproject.toml" | "package.json")
    )
}

#[derive(Default)]
struct ImplementationBuilder {
    implementation_types: BTreeSet<String>,
    runtime_types: BTreeSet<String>,
    runtime_manifests: Vec<String>,
    source_files: Vec<String>,
}

fn implementations(
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
) -> Vec<Implementation> {
    let mut builders: BTreeMap<String, ImplementationBuilder> = BTreeMap::new();
    for manifest in manifests
        .keys()
        .filter(|manifest| establishes_implementation_ownership(Path::new(manifest)))
    {
        let path = Path::new(manifest);
        let root = path
            .parent()
            .and_then(Path::to_str)
            .unwrap_or_default()
            .to_string();
        let (implementation_type, runtime_type) =
            runtime_manifest(path).expect("captured runtime manifest");
        let builder = builders.entry(root).or_default();
        builder
            .implementation_types
            .insert(implementation_type.to_string());
        builder.runtime_types.insert(runtime_type.to_string());
        builder.runtime_manifests.push(manifest.clone());
    }
    let roots: Vec<_> = builders.keys().cloned().collect();
    for manifest in manifests
        .keys()
        .filter(|manifest| !establishes_implementation_ownership(Path::new(manifest)))
    {
        let path = Path::new(manifest);
        let owner = roots
            .iter()
            .filter(|root| root.is_empty() || path.starts_with(Path::new(root)))
            .max_by_key(|root| Path::new(root).components().count());
        if let Some(root) = owner {
            builders
                .get_mut(root)
                .expect("captured implementation root")
                .runtime_manifests
                .push(manifest.clone());
        }
    }
    let mut unowned = vec![];
    for file in files.keys() {
        let path = Path::new(file);
        let owner = roots
            .iter()
            .filter(|root| root.is_empty() || path.starts_with(Path::new(root)))
            .max_by_key(|root| Path::new(root).components().count());
        if let Some(root) = owner {
            builders
                .get_mut(root)
                .expect("captured implementation root")
                .source_files
                .push(file.clone());
        } else {
            unowned.push(file.clone());
        }
    }
    let mut result: Vec<_> = builders
        .into_iter()
        .filter(|(_, builder)| !builder.source_files.is_empty())
        .map(|(root, builder)| Implementation {
            id: if root.is_empty() {
                ".".into()
            } else {
                root.clone()
            },
            root: Some(if root.is_empty() { ".".into() } else { root }),
            ownership: "runtime_manifest",
            implementation_types: builder.implementation_types.into_iter().collect(),
            runtime_types: builder.runtime_types.into_iter().collect(),
            runtime_manifests: builder.runtime_manifests,
            source_files: builder.source_files,
        })
        .collect();
    if !unowned.is_empty() {
        result.push(Implementation {
            id: "__unowned__".into(),
            root: None,
            ownership: "unowned_source",
            implementation_types: vec!["unowned_source".into()],
            runtime_types: vec![],
            runtime_manifests: vec![],
            source_files: unowned,
        });
    }
    result
}

/// What corpus capture records: source files, runtime manifests and the
/// directory symlinks whose targets stay inside the root.
#[derive(Default)]
struct CapturedCorpus {
    files: BTreeMap<String, String>,
    manifests: BTreeMap<String, String>,
    directory_links: BTreeMap<String, String>,
}

fn walk(
    root: &Path,
    dir: &Path,
    policy: &Policy,
    registry: &Registry,
    corpus: &mut CapturedCorpus,
) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("cannot enumerate source directory: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot enumerate source entry: {e}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        walk_entry(root, &entry, policy, registry, corpus)?;
    }
    Ok(())
}

fn reject_symlink(path: &Path, relative: &Path, registry: &Registry) -> Result<(), String> {
    if source_file(path, &registry.language) {
        return Err(format!("symlink in source corpus: {}", relative.display()));
    }
    if captured_manifest(relative, registry) {
        return Err(format!("symlink runtime manifest: {}", relative.display()));
    }
    Ok(())
}

/// The root-relative target of a directory symlink, or `None` when the link
/// does not resolve to a directory inside `root`. The walk never follows the
/// link; module resolution maps paths through it to files captured at the
/// target, so each file is scanned once.
fn in_root_directory_link(root: &Path, path: &Path) -> Option<String> {
    let target = fs::canonicalize(path).ok()?;
    if !target.is_dir() {
        return None;
    }
    let relative = target.strip_prefix(root).ok()?.to_str()?;
    (!relative.is_empty() && !relative.contains('\\')).then(|| relative.to_string())
}

fn capture_working_file(
    path: &Path,
    relative: &Path,
    policy: &Policy,
    corpus: &mut CapturedCorpus,
    is_manifest: bool,
) -> Result<(), String> {
    let name = relative
        .to_str()
        .ok_or("source path is not UTF-8")?
        .to_string();
    if name.contains('\\') {
        return Err("backslash source paths are not supported in the portable corpus".into());
    }
    let bytes = fs::read(path).map_err(|e| format!("cannot read {name}: {e}"))?;
    let source = text(bytes, &name)?;
    if is_manifest {
        corpus.manifests.insert(name, source);
    } else {
        if corpus.files.len() >= policy.limits.maximum_files {
            return Err("maximum_files budget exceeded".into());
        }
        corpus.files.insert(name, source);
    }
    Ok(())
}

fn walk_entry(
    root: &Path,
    entry: &fs::DirEntry,
    policy: &Policy,
    registry: &Registry,
    corpus: &mut CapturedCorpus,
) -> Result<(), String> {
    let path = entry.path();
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "source path escapes root")?;
    if excluded(relative, policy) {
        return Ok(());
    }
    let kind = entry
        .file_type()
        .map_err(|e| format!("cannot inspect source entry: {e}"))?;
    if kind.is_symlink() {
        return capture_working_link(root, &path, relative, registry, corpus);
    }
    if kind.is_dir() {
        return walk(root, &path, policy, registry, corpus);
    }
    let is_manifest = captured_manifest(relative, registry);
    if source_file(&path, &registry.language) || is_manifest {
        capture_working_file(&path, relative, policy, corpus, is_manifest)?;
    }
    Ok(())
}

/// Rejects a symlinked source file and records an in-root directory link.
fn capture_working_link(
    root: &Path,
    path: &Path,
    relative: &Path,
    registry: &Registry,
    corpus: &mut CapturedCorpus,
) -> Result<(), String> {
    reject_symlink(path, relative, registry)?;
    if let (Some(link), Some(target)) = (relative.to_str(), in_root_directory_link(root, path)) {
        corpus.directory_links.insert(link.to_string(), target);
    }
    Ok(())
}

struct ArtifactRecognizer<'a> {
    rule_pack: &'a str,
    files: &'a BTreeMap<String, String>,
    manifests: &'a BTreeMap<String, String>,
    implementations: &'a [Implementation],
    index: &'a crate::migration::MigrationArtifactIndex,
}

impl ArtifactRecognizer<'_> {
    fn recognizes(&self, path: &str) -> bool {
        crate::migration::is_recognized_migration_artifact(
            self.rule_pack,
            self.files,
            self.manifests,
            self.index,
            self.implementations,
            path,
        )
    }
}

fn maximum_file_budget_reached(files: usize, artifacts: usize, maximum: usize) -> bool {
    files.saturating_add(artifacts) >= maximum
}

fn walk_migration_artifacts(
    root: &Path,
    dir: &Path,
    policy: &Policy,
    registry: &Registry,
    recognizer: &ArtifactRecognizer<'_>,
    artifacts: &mut BTreeMap<String, Vec<u8>>,
    candidates: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("cannot enumerate migration artifact directory: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot enumerate migration artifact entry: {e}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        walk_migration_artifact_entry(
            root, &entry, policy, registry, recognizer, artifacts, candidates,
        )?;
    }
    Ok(())
}

fn walk_migration_artifact_entry(
    root: &Path,
    entry: &fs::DirEntry,
    policy: &Policy,
    registry: &Registry,
    recognizer: &ArtifactRecognizer<'_>,
    artifacts: &mut BTreeMap<String, Vec<u8>>,
    candidates: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let path = entry.path();
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "migration artifact path escapes root")?;
    if excluded(relative, policy) {
        return Ok(());
    }
    let kind = entry
        .file_type()
        .map_err(|error| format!("cannot inspect migration artifact entry: {error}"))?;
    if kind.is_dir() {
        return walk_migration_artifacts(
            root, &path, policy, registry, recognizer, artifacts, candidates,
        );
    }
    capture_migration_artifact(
        &path,
        relative,
        kind.is_symlink(),
        policy,
        registry,
        recognizer,
        artifacts,
        candidates,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "git capture threads repository, path, and evidence context separately"
)]
fn capture_migration_artifact(
    path: &Path,
    relative: &Path,
    symlink: bool,
    policy: &Policy,
    registry: &Registry,
    recognizer: &ArtifactRecognizer<'_>,
    artifacts: &mut BTreeMap<String, Vec<u8>>,
    candidates: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    if !migration_artifact_candidate(relative, registry) {
        return Ok(());
    }
    let name = relative
        .to_str()
        .ok_or("migration artifact path is not UTF-8")?;
    if !recognizer.recognizes(name) {
        return capture_unresolved_migration_candidate(
            path,
            name,
            symlink,
            policy,
            recognizer.files.len(),
            artifacts.len(),
            candidates,
        );
    }
    capture_recognized_migration_artifact(
        path,
        name,
        symlink,
        policy,
        recognizer.files.len(),
        artifacts,
    )
}

fn capture_unresolved_migration_candidate(
    path: &Path,
    name: &str,
    symlink: bool,
    policy: &Policy,
    source_count: usize,
    artifact_count: usize,
    candidates: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    if source_count
        .saturating_add(artifact_count)
        .saturating_add(candidates.len())
        >= policy.limits.maximum_files
    {
        return Err("maximum_files budget exceeded".into());
    }
    if symlink {
        return Err(format!("symlink migration artifact candidate: {name}"));
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read migration artifact candidate {name}: {error}"))?;
    candidates.insert(name.to_string(), bytes);
    Ok(())
}

fn capture_recognized_migration_artifact(
    path: &Path,
    name: &str,
    symlink: bool,
    policy: &Policy,
    source_count: usize,
    artifacts: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    if symlink {
        return Err(format!("symlink migration artifact: {name}"));
    }
    if maximum_file_budget_reached(source_count, artifacts.len(), policy.limits.maximum_files) {
        return Err("maximum_files budget exceeded".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("cannot read {name}: {error}"))?;
    artifacts.insert(name.to_string(), bytes);
    Ok(())
}

fn snapshot_digest(snapshot: &CapturedSnapshot, policy: &str, resolved_selection: &[u8]) -> String {
    let CapturedSnapshot {
        files,
        manifests,
        directory_links,
        artifacts,
        artifact_candidates,
        history,
        implementations: _,
    } = snapshot;
    let mut parts = vec![
        policy.as_bytes(),
        b"resolved_policy_selection",
        resolved_selection,
    ];
    for (path, source) in manifests {
        parts.push(b"runtime_manifest");
        parts.push(path.as_bytes());
        parts.push(source.as_bytes());
    }
    for (link, target) in directory_links {
        parts.push(b"directory_link");
        parts.push(link.as_bytes());
        parts.push(target.as_bytes());
    }
    for (path, source) in files {
        parts.push(b"source");
        parts.push(path.as_bytes());
        parts.push(source.as_bytes());
    }
    for (path, source) in artifacts {
        parts.push(b"migration_artifact");
        parts.push(path.as_bytes());
        parts.push(source.as_slice());
    }
    for (path, source) in artifact_candidates {
        parts.push(b"migration_artifact_candidate");
        parts.push(path.as_bytes());
        parts.push(source.as_slice());
    }
    parts.push(b"git_history_available");
    parts.push(if history.available { b"true" } else { b"false" });
    for commit in &history.commits {
        parts.push(if commit.root {
            b"git_root_commit".as_slice()
        } else {
            b"git_commit".as_slice()
        });
        parts.push(commit.id.as_bytes());
        for path in &commit.files {
            parts.push(path.as_bytes());
        }
    }
    digest(&parts)
}

fn reports_inside_work_tree(success: bool, stdout: &[u8]) -> bool {
    success && stdout == b"true\n"
}

fn captured_history(root: &Path, files: &BTreeMap<String, String>) -> History {
    captured_history_at(root, None, files)
}

fn captured_history_at(
    root: &Path,
    revision: Option<&str>,
    files: &BTreeMap<String, String>,
) -> History {
    let inside = Command::new("git")
        .arg("--no-replace-objects")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .output();
    if !inside.is_ok_and(|output| reports_inside_work_tree(output.status.success(), &output.stdout))
    {
        return History::default();
    }
    let target = revision.unwrap_or("HEAD");
    let head = Command::new("git")
        .arg("--no-replace-objects")
        .args(["rev-parse", "--verify", target])
        .current_dir(root)
        .output();
    if head.is_ok_and(|output| !output.status.success()) {
        return History {
            available: revision.is_none(),
            shallow: false,
            commits: Vec::new(),
        };
    }
    let Some(shallow) = shallow_repository(root) else {
        return History::default();
    };
    let limit = HISTORY_WINDOW.to_string();
    let mut args = vec![
        "-c",
        "core.quotepath=false",
        "log",
        "--format=%x1e%H %P%x1f%s",
        "--name-only",
        "--no-renames",
        "--relative",
        "-n",
        &limit,
    ];
    if let Some(commit) = revision {
        args.push(commit);
    }
    args.extend(["--", "."]);
    let output = Command::new("git")
        .arg("--no-replace-objects")
        .args(&args)
        .current_dir(root)
        .output();
    let Ok(output) = output else {
        return History::default();
    };
    if !output.status.success() {
        return History::default();
    }
    let Ok(log) = String::from_utf8(output.stdout) else {
        return History::default();
    };
    let mut history = parse_history_log(&log, files, shallow);
    if !history.commits.is_empty() {
        let suppression_only = suppression_only_edits(root, revision, &limit, files);
        without_suppression_edits(&mut history, &suppression_only);
    }
    history
}

/// Drops file changes that only edit suppressions, and commits left with none.
fn without_suppression_edits(history: &mut History, suppression_only: &BTreeSet<(String, String)>) {
    for commit in &mut history.commits {
        commit
            .files
            .retain(|path| !suppression_only.contains(&(commit.id.clone(), path.clone())));
    }
    history.commits.retain(|commit| !commit.files.is_empty());
}

fn parse_history_log(log: &str, files: &BTreeMap<String, String>, shallow: bool) -> History {
    let blocks = log
        .split('\u{1e}')
        .filter(|block| !block.trim().is_empty())
        .collect::<Vec<_>>();
    if shallow_window_truncated(shallow, blocks.len()) {
        return History {
            available: false,
            shallow: true,
            commits: Vec::new(),
        };
    }
    let mut commits = Vec::new();
    for block in blocks {
        let mut lines = block.lines().filter(|line| !line.trim().is_empty());
        let Some(header) = lines.next() else {
            continue;
        };
        let (ids, subject) = header.split_once('\u{1f}').unwrap_or((header, ""));
        let mut ids = ids.split_whitespace();
        let Some(id) = ids.next() else {
            continue;
        };
        // A parentless commit is an initial import. The oldest commit of a shallow
        // clone also has no parents here, which is only safe because a shallow
        // boundary inside the history window marks history unavailable.
        let root = ids.next().is_none();
        let mut changed = lines
            .map(str::trim)
            .filter(|path| files.contains_key(*path))
            .map(str::to_string)
            .collect::<Vec<_>>();
        changed.sort();
        changed.dedup();
        if !changed.is_empty() {
            commits.push(HistoryCommit {
                id: id.into(),
                root,
                subject: subject.trim().into(),
                files: changed,
            });
        }
    }
    History {
        available: true,
        shallow: false,
        commits,
    }
}

/// Reports whether the repository is a shallow clone. `None` means Git could
/// not answer, which callers treat as unavailable history rather than complete.
fn shallow_repository(root: &Path) -> Option<bool> {
    let output = Command::new("git")
        .arg("--no-replace-objects")
        .args(["rev-parse", "--is-shallow-repository"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    match output.stdout.as_slice() {
        b"true\n" => Some(true),
        b"false\n" => Some(false),
        _ => None,
    }
}

/// A shallow boundary only truncates the bounded window when Git returned
/// fewer commits than requested; a full window is complete regardless of depth.
fn shallow_window_truncated(shallow: bool, logged: usize) -> bool {
    shallow && logged < HISTORY_WINDOW
}

pub(crate) fn working_tree(
    root: &Path,
    policy_path: &Path,
    selection: &SelectionOptions,
) -> Result<CapturedInput, String> {
    let policy_text = text(
        fs::read(policy_path).map_err(|e| format!("cannot read policy: {e}"))?,
        "policy",
    )?;
    working_tree_with_policy_text(root, &policy_text, selection)
}

pub(crate) fn working_tree_with_policy_text(
    root: &Path,
    policy_text: &str,
    selection: &SelectionOptions,
) -> Result<CapturedInput, String> {
    let root = working_root(root)?;
    let registry = crate::policy::registry_for_policy(policy_text)?;
    let policy = crate::policy::parse_with_selection(policy_text, &registry, selection)?;
    let snapshot = working_snapshot(&root, &policy, &registry)?;
    captured_input(
        snapshot,
        policy_text,
        policy,
        registry,
        "working_tree",
        "source corpus",
    )
}

fn working_root(root: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root).map_err(|e| format!("cannot open source root: {e}"))?;
    if root.is_dir() {
        Ok(root)
    } else {
        Err("--path must be a source directory".into())
    }
}

fn working_snapshot(
    root: &Path,
    policy: &Policy,
    registry: &Registry,
) -> Result<CapturedSnapshot, String> {
    let mut corpus = CapturedCorpus::default();
    walk(root, root, policy, registry, &mut corpus)?;
    let CapturedCorpus {
        files,
        manifests,
        directory_links,
    } = corpus;
    let history = captured_history(root, &files);
    let implementations = implementations(&files, &manifests);
    let artifact_index = crate::migration::migration_artifact_index(
        &policy.rule_pack,
        &files,
        &manifests,
        &implementations,
    );
    let recognizer = ArtifactRecognizer {
        rule_pack: &policy.rule_pack,
        files: &files,
        manifests: &manifests,
        implementations: &implementations,
        index: &artifact_index,
    };
    let mut artifacts = BTreeMap::new();
    let mut artifact_candidates = BTreeMap::new();
    walk_migration_artifacts(
        root,
        root,
        policy,
        registry,
        &recognizer,
        &mut artifacts,
        &mut artifact_candidates,
    )?;
    Ok(CapturedSnapshot {
        files,
        manifests,
        directory_links,
        implementations,
        artifacts,
        artifact_candidates,
        history,
    })
}

type IndexEntries = BTreeMap<String, (String, String)>;

fn validate_staged_policy_path(policy_path: &Path) -> Result<&str, String> {
    if policy_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(
            "staged policy must be a repository-relative path without escaping components".into(),
        );
    }
    policy_path
        .to_str()
        .ok_or("policy path is not UTF-8".into())
}

pub(crate) fn repository_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    repository_root_of(&cwd)
}

/// The top folder of the Git worktree that holds `folder`, as Git reports it.
pub(crate) fn repository_root_of(folder: &Path) -> Result<PathBuf, String> {
    let root_text = text(git(folder, &["rev-parse", "--show-toplevel"])?, "Git root")?;
    Ok(PathBuf::from(
        root_text.strip_suffix('\n').unwrap_or(&root_text),
    ))
}

fn parse_index(initial: &[u8]) -> Result<IndexEntries, String> {
    let mut entries = BTreeMap::new();
    for entry in initial.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let entry = std::str::from_utf8(entry).map_err(|_| "index contains non-UTF-8 paths")?;
        let (header, path) = entry.split_once('\t').ok_or("malformed index entry")?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || fields[2] != "0" {
            return Err("unmerged or malformed staged snapshot".into());
        }
        if entries
            .insert(
                path.to_string(),
                (fields[0].to_string(), fields[1].to_string()),
            )
            .is_some()
        {
            return Err("duplicate staged path".into());
        }
    }
    Ok(entries)
}

fn staged_policy(
    root: &Path,
    entries: &IndexEntries,
    name: &str,
    selection: &SelectionOptions,
) -> Result<(String, Registry, Policy), String> {
    let (mode, oid) = entries
        .get(name)
        .ok_or("policy is not staged; refusing unstaged policy")?;
    if !matches!(mode.as_str(), "100644" | "100755") {
        return Err("staged policy is not a regular file".into());
    }
    let policy_text = text(git(root, &["cat-file", "blob", oid])?, name)?;
    let registry = crate::policy::registry_for_policy(&policy_text)?;
    let policy = crate::policy::parse_with_selection(&policy_text, &registry, selection)?;
    Ok((policy_text, registry, policy))
}

fn validate_staged_path(name: &str, path: &Path) -> Result<(), String> {
    if name.contains('\\')
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("non-portable or escaping staged source path".into());
    }
    Ok(())
}

fn capture_staged_blob(
    root: &Path,
    name: &str,
    mode: &str,
    oid: &str,
    is_manifest: bool,
) -> Result<String, String> {
    if !matches!(mode, "100644" | "100755") {
        return Err(if is_manifest {
            format!("staged runtime manifest is not a regular file: {name}")
        } else {
            format!("staged source is not a regular file: {name}")
        });
    }
    text(git(root, &["cat-file", "blob", oid])?, name)
}

fn capture_staged_artifact(
    root: &Path,
    name: &str,
    mode: &str,
    oid: &str,
) -> Result<Vec<u8>, String> {
    if !matches!(mode, "100644" | "100755") {
        return Err(format!(
            "staged migration artifact is not a regular file: {name}"
        ));
    }
    git(root, &["cat-file", "blob", oid])
}

fn staged_corpus(
    root: &Path,
    entries: &IndexEntries,
    registry: &Registry,
    policy: &Policy,
) -> Result<CapturedCorpus, String> {
    let mut corpus = CapturedCorpus::default();
    for (name, (mode, oid)) in entries {
        let path = Path::new(name);
        if excluded(path, policy) {
            continue;
        }
        if !source_file(path, &registry.language) && !captured_manifest(path, registry) {
            if mode == "120000" {
                capture_staged_link(root, name, oid, entries, &mut corpus)?;
            }
            continue;
        }
        validate_staged_path(name, path)?;
        let is_manifest = captured_manifest(path, registry);
        let source = capture_staged_blob(root, name, mode, oid, is_manifest)?;
        record_staged_source(&mut corpus, name, source, is_manifest, policy)?;
    }
    Ok(corpus)
}

/// Adds a captured staged blob as a manifest or a source file, within the
/// `maximum_files` budget.
fn record_staged_source(
    corpus: &mut CapturedCorpus,
    name: &str,
    source: String,
    is_manifest: bool,
    policy: &Policy,
) -> Result<(), String> {
    if is_manifest {
        corpus.manifests.insert(name.to_string(), source);
        return Ok(());
    }
    if corpus.files.len() >= policy.limits.maximum_files {
        return Err("maximum_files budget exceeded".into());
    }
    corpus.files.insert(name.to_string(), source);
    Ok(())
}

/// Records the staged symlink at `name` when it names a staged directory.
fn capture_staged_link(
    root: &Path,
    name: &str,
    oid: &str,
    entries: &IndexEntries,
    corpus: &mut CapturedCorpus,
) -> Result<(), String> {
    let target = text(git(root, &["cat-file", "blob", oid])?, name)?;
    if let Some(target) = staged_directory_link(name, &target, entries) {
        corpus.directory_links.insert(name.to_string(), target);
    }
    Ok(())
}

/// The repository-relative directory a staged symlink at `name` points to, or
/// `None` when the target leaves the repository, is absolute, or names no
/// directory of staged entries.
fn staged_directory_link(name: &str, target: &str, entries: &IndexEntries) -> Option<String> {
    let mut resolved: Vec<&str> = name.split('/').collect();
    resolved.pop();
    if target.starts_with('/') || target.contains('\\') {
        return None;
    }
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                resolved.pop()?;
            }
            segment => resolved.push(segment),
        }
    }
    let target = resolved.join("/");
    let prefix = format!("{target}/");
    (!target.is_empty()
        && entries
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(entry, _)| entry.starts_with(&prefix)))
    .then_some(target)
}

fn staged_migration_artifacts(
    root: &Path,
    entries: &IndexEntries,
    registry: &Registry,
    policy: &Policy,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementations: &[Implementation],
) -> Result<CapturedMigrationArtifacts, String> {
    let artifact_index = crate::migration::migration_artifact_index(
        &policy.rule_pack,
        files,
        manifests,
        implementations,
    );
    let recognizer = ArtifactRecognizer {
        rule_pack: &policy.rule_pack,
        files,
        manifests,
        implementations,
        index: &artifact_index,
    };
    let mut artifacts = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    for (name, (mode, oid)) in entries {
        capture_staged_migration_artifact(
            root,
            name,
            mode,
            oid,
            registry,
            policy,
            &recognizer,
            files.len(),
            &mut artifacts,
            &mut candidates,
        )?;
    }
    Ok((artifacts, candidates))
}

#[expect(
    clippy::too_many_arguments,
    reason = "git capture threads repository, path, and evidence context separately"
)]
fn capture_staged_migration_artifact(
    root: &Path,
    name: &str,
    mode: &str,
    oid: &str,
    registry: &Registry,
    policy: &Policy,
    recognizer: &ArtifactRecognizer<'_>,
    source_count: usize,
    artifacts: &mut BTreeMap<String, Vec<u8>>,
    candidates: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let path = Path::new(name);
    if excluded(path, policy) || !migration_artifact_candidate(path, registry) {
        return Ok(());
    }
    validate_staged_path(name, path)?;
    if !recognizer.recognizes(name) {
        return capture_staged_candidate(
            root,
            name,
            mode,
            oid,
            policy,
            source_count,
            artifacts.len(),
            candidates,
        );
    }
    if maximum_file_budget_reached(source_count, artifacts.len(), policy.limits.maximum_files) {
        return Err("maximum_files budget exceeded".into());
    }
    artifacts.insert(
        name.to_string(),
        capture_staged_artifact(root, name, mode, oid)?,
    );
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "git capture threads repository, path, and evidence context separately"
)]
fn capture_staged_candidate(
    root: &Path,
    name: &str,
    mode: &str,
    oid: &str,
    policy: &Policy,
    source_count: usize,
    artifact_count: usize,
    candidates: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    if source_count
        .saturating_add(artifact_count)
        .saturating_add(candidates.len())
        >= policy.limits.maximum_files
    {
        return Err("maximum_files budget exceeded".into());
    }
    candidates.insert(
        name.to_string(),
        capture_staged_artifact(root, name, mode, oid)?,
    );
    Ok(())
}

fn validate_stable_index(root: &Path, initial: &[u8]) -> Result<(), String> {
    if initial == git(root, &["ls-files", "--stage", "-z"])? {
        Ok(())
    } else {
        Err("Git index changed during snapshot capture".into())
    }
}

struct CapturedSnapshot {
    files: BTreeMap<String, String>,
    manifests: BTreeMap<String, String>,
    directory_links: BTreeMap<String, String>,
    implementations: Vec<Implementation>,
    artifacts: BTreeMap<String, Vec<u8>>,
    artifact_candidates: BTreeMap<String, Vec<u8>>,
    history: History,
}

fn captured_input(
    snapshot: CapturedSnapshot,
    policy_text: &str,
    policy: Policy,
    registry: Registry,
    mode: &'static str,
    corpus_label: &str,
) -> Result<CapturedInput, String> {
    let snapshot_sha256 = snapshot_digest(
        &snapshot,
        policy_text,
        &serde_json::to_vec(&policy.resolved).expect("resolved selection serializes"),
    );
    let CapturedSnapshot {
        files,
        manifests,
        directory_links,
        implementations,
        artifacts,
        artifact_candidates,
        history,
    } = snapshot;
    if files.is_empty() {
        return Err(format!(
            "{corpus_label} contains no {} files",
            registry.language
        ));
    }
    let source_roles = crate::migration::resolve_source_roles(
        &policy,
        &files,
        &manifests,
        &registry.language,
        &implementations,
    )?;
    Ok(CapturedInput {
        input: Input {
            policy_sha256: digest(&[policy_text.as_bytes()]),
            digest: snapshot_sha256,
            manifests,
            directory_links,
            implementations,
            source_roles,
            history,
            artifacts,
            artifact_candidates,
            files,
            policy,
            mode,
            coverage: None,
        },
        registry,
    })
}

fn staged_evidence(
    root: &Path,
    entries: &IndexEntries,
    evidence_path: Option<&Path>,
) -> Result<Option<Vec<u8>>, String> {
    let Some(path) = evidence_path else {
        return Ok(None);
    };
    let name = validate_staged_policy_path(path)?;
    let (mode, oid) = entries.get(name).ok_or("provider evidence is not staged")?;
    if !matches!(mode.as_str(), "100644" | "100755") {
        return Err("staged provider evidence is not a regular file".into());
    }
    Ok(Some(git(root, &["cat-file", "blob", oid])?))
}

struct StagedCapture {
    root: PathBuf,
    initial: Vec<u8>,
    entries: IndexEntries,
    policy_text: String,
    registry: Registry,
    policy: Policy,
}

fn staged_capture(
    policy_path: &Path,
    selection: &SelectionOptions,
) -> Result<StagedCapture, String> {
    let policy_name = validate_staged_policy_path(policy_path)?;
    let root = repository_root()?;
    let initial = git(&root, &["ls-files", "--stage", "-z"])?;
    let entries = parse_index(&initial)?;
    let (policy_text, registry, policy) = staged_policy(&root, &entries, policy_name, selection)?;
    Ok(StagedCapture {
        root,
        initial,
        entries,
        policy_text,
        registry,
        policy,
    })
}

fn staged_snapshot(capture: &StagedCapture) -> Result<CapturedSnapshot, String> {
    let CapturedCorpus {
        files,
        manifests,
        directory_links,
    } = staged_corpus(
        &capture.root,
        &capture.entries,
        &capture.registry,
        &capture.policy,
    )?;
    let implementations = implementations(&files, &manifests);
    let (artifacts, artifact_candidates) = staged_migration_artifacts(
        &capture.root,
        &capture.entries,
        &capture.registry,
        &capture.policy,
        &files,
        &manifests,
        &implementations,
    )?;
    let history = captured_history(&capture.root, &files);
    Ok(CapturedSnapshot {
        files,
        manifests,
        directory_links,
        implementations,
        artifacts,
        artifact_candidates,
        history,
    })
}

pub(crate) fn staged(
    policy_path: &Path,
    evidence_path: Option<&Path>,
    selection: &SelectionOptions,
) -> Result<(CapturedInput, Option<Vec<u8>>), String> {
    let capture = staged_capture(policy_path, selection)?;
    let snapshot = staged_snapshot(&capture)?;
    let evidence = staged_evidence(&capture.root, &capture.entries, evidence_path)?;
    validate_stable_index(&capture.root, &capture.initial)?;
    let input = captured_input(
        snapshot,
        &capture.policy_text,
        capture.policy,
        capture.registry,
        "staged_snapshot",
        "staged corpus",
    )?;
    Ok((input, evidence))
}

fn parse_tree(raw: &[u8]) -> Result<IndexEntries, String> {
    let mut entries = BTreeMap::new();
    for entry in raw.split(|byte| *byte == 0).filter(|item| !item.is_empty()) {
        let entry = std::str::from_utf8(entry).map_err(|_| "tree contains non-UTF-8 paths")?;
        let (header, path) = entry.split_once('\t').ok_or("malformed tree entry")?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 {
            return Err("malformed tree entry".into());
        }
        if fields[1] != "blob" {
            continue;
        }
        if entries
            .insert(
                path.to_string(),
                (fields[0].to_string(), fields[2].to_string()),
            )
            .is_some()
        {
            return Err("duplicate tree path".into());
        }
    }
    Ok(entries)
}

fn resolve_git_commit(root: &Path, reference_ref: &str) -> Result<String, String> {
    let spec = format!("{reference_ref}^{{commit}}");
    let raw = git(root, &["rev-parse", "--verify", &spec])
        .map_err(|error| format!("cannot resolve base reference {reference_ref:?}: {error}"))?;
    let text = text(raw, "base reference")?;
    Ok(text.trim().to_string())
}

fn reference_policy_at_commit(
    root: &Path,
    entries: &IndexEntries,
    policy_rel_path: Option<&str>,
    selection: &SelectionOptions,
) -> (Option<Policy>, Option<String>) {
    let Some(name) = policy_rel_path else {
        return (None, None);
    };
    let Some((mode, oid)) = entries.get(name) else {
        return (None, None);
    };
    if !matches!(mode.as_str(), "100644" | "100755") {
        return (None, None);
    }
    let Ok(bytes) = git(root, &["cat-file", "blob", oid]) else {
        return (None, None);
    };
    let Ok(policy_text) = text(bytes, name) else {
        return (None, None);
    };
    let sha = digest(&[policy_text.as_bytes()]);
    let parsed = crate::policy::registry_for_policy(&policy_text)
        .and_then(|reg| crate::policy::parse_with_selection(&policy_text, &reg, selection))
        .ok();
    (parsed, Some(sha))
}

fn capture_git_ref(
    root: &Path,
    reference_ref: &str,
    policy_rel_path: Option<&str>,
    candidate_policy_text: &str,
    selection: &SelectionOptions,
) -> Result<crate::core::model::ReferenceSnapshot, String> {
    let reference_commit = resolve_git_commit(root, reference_ref)?;
    let raw_tree = git(root, &["ls-tree", "-r", "-z", &reference_commit])?;
    let entries = parse_tree(&raw_tree)?;
    let (reference_policy, reference_policy_sha256) =
        reference_policy_at_commit(root, &entries, policy_rel_path, selection);
    let captured = capture_reference_input(
        root,
        &reference_commit,
        &entries,
        candidate_policy_text,
        selection,
    )?;
    Ok(crate::core::model::ReferenceSnapshot {
        reference_ref: reference_ref.to_string(),
        reference_commit,
        captured,
        reference_policy,
        reference_policy_sha256,
    })
}

fn capture_reference_input(
    root: &Path,
    reference_commit: &str,
    entries: &IndexEntries,
    candidate_policy_text: &str,
    selection: &SelectionOptions,
) -> Result<Option<CapturedInput>, String> {
    let registry = crate::policy::registry_for_policy(candidate_policy_text)?;
    let policy = crate::policy::parse_with_selection(candidate_policy_text, &registry, selection)?;
    let CapturedCorpus {
        files,
        manifests,
        directory_links,
    } = staged_corpus(root, entries, &registry, &policy)?;
    if files.is_empty() {
        return Ok(None);
    }
    let implementations = implementations(&files, &manifests);
    let (artifacts, artifact_candidates) = staged_migration_artifacts(
        root,
        entries,
        &registry,
        &policy,
        &files,
        &manifests,
        &implementations,
    )?;
    let history = captured_history_at(root, Some(reference_commit), &files);
    let snapshot = CapturedSnapshot {
        files,
        manifests,
        directory_links,
        implementations,
        artifacts,
        artifact_candidates,
        history,
    };
    captured_input(
        snapshot,
        candidate_policy_text,
        policy,
        registry,
        "git_reference",
        "reference corpus",
    )
    .map(Some)
}

pub(crate) fn git_ref_for_staged(
    reference_ref: &str,
    policy_path: &Path,
    selection: &SelectionOptions,
) -> Result<crate::core::model::ReferenceSnapshot, String> {
    let capture = staged_capture(policy_path, selection)?;
    let rel = validate_staged_policy_path(policy_path)?;
    capture_git_ref(
        &capture.root,
        reference_ref,
        Some(rel),
        &capture.policy_text,
        selection,
    )
}

pub(crate) fn git_ref_for_working_tree(
    source: &Path,
    reference_ref: &str,
    policy_path: &Path,
    selection: &SelectionOptions,
) -> Result<crate::core::model::ReferenceSnapshot, String> {
    let root = working_root(source)?;
    let policy_text = text(
        fs::read(policy_path).map_err(|e| format!("cannot read policy: {e}"))?,
        "policy",
    )?;
    let rel = fs::canonicalize(policy_path)
        .ok()
        .and_then(|canonical| canonical.strip_prefix(&root).ok().map(Path::to_path_buf))
        .and_then(|relative| relative.to_str().map(str::to_string));
    capture_git_ref(
        &root,
        reference_ref,
        rel.as_deref(),
        &policy_text,
        selection,
    )
}

pub(crate) fn git_ref_for_working_tree_with_policy_text(
    source: &Path,
    reference_ref: &str,
    policy_text: &str,
    selection: &SelectionOptions,
) -> Result<crate::core::model::ReferenceSnapshot, String> {
    let root = working_root(source)?;
    capture_git_ref(&root, reference_ref, None, policy_text, selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[cfg(unix)]
    #[test]
    fn working_directory_links_must_name_a_directory_strictly_inside_the_root() {
        let repository = TempRepository::new();
        let root = fs::canonicalize(&repository.0).unwrap();
        fs::create_dir(root.join("common")).unwrap();
        fs::write(root.join("common/mod.rs"), "").unwrap();
        let link = |name: &str, target: &Path| {
            std::os::unix::fs::symlink(target, root.join(name)).unwrap();
            in_root_directory_link(&root, &root.join(name))
        };
        assert_eq!(link("linked", Path::new("common")), Some("common".into()));
        assert_eq!(link("itself", Path::new(".")), None);
        assert_eq!(link("file", Path::new("common/mod.rs")), None);
        assert_eq!(link("outside", &std::env::temp_dir()), None);
        assert_eq!(link("dangling", Path::new("absent")), None);
    }

    #[test]
    fn staged_directory_links_must_name_a_staged_directory_inside_the_repository() {
        let entry = || ("100644".to_string(), "oid".to_string());
        let entries = IndexEntries::from([
            ("common/mod.rs".into(), entry()),
            ("src/lib.rs".into(), entry()),
        ]);
        let link = |name, target| staged_directory_link(name, target, &entries);
        assert_eq!(link("src/linked", "../common"), Some("common".into()));
        assert_eq!(link("src/linked", "./../common/"), Some("common".into()));
        assert_eq!(link("linked", "common"), Some("common".into()));
        assert_eq!(link("src/linked", "../../common"), None);
        assert_eq!(link("src/linked", "/common"), None);
        assert_eq!(link("linked", "/common"), None);
        assert_eq!(link("linked", "common\\"), None);
        assert_eq!(link("src/linked", "..\\common"), None);
        assert_eq!(link("src/linked", ".."), None);
        assert_eq!(link("src/linked", "../comm"), None);
        assert_eq!(link("src/linked", "../common/mod.rs"), None);
        assert_eq!(link("src/linked", "../zzz"), None);
    }

    #[test]
    fn shallow_boundary_only_truncates_a_partial_history_window() {
        assert!(shallow_window_truncated(true, 0));
        assert!(shallow_window_truncated(true, HISTORY_WINDOW - 1));
        assert!(!shallow_window_truncated(true, HISTORY_WINDOW));
        assert!(!shallow_window_truncated(false, 0));
    }

    struct TempRepository(PathBuf);

    impl TempRepository {
        fn new() -> Self {
            let unique = format!(
                "smells-index-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir(&path).unwrap();
            let output = Command::new("git")
                .args(["init", "-q"])
                .current_dir(&path)
                .output()
                .unwrap();
            assert!(output.status.success());
            Self(path)
        }

        fn stage(&self, contents: &str) {
            fs::write(self.0.join("tracked.txt"), contents).unwrap();
            let output = Command::new("git")
                .args(["add", "--", "tracked.txt"])
                .current_dir(&self.0)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
    }

    impl Drop for TempRepository {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn run_git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn commit_file(root: &Path, contents: &str, message: &str) -> String {
        fs::write(root.join("a.ts"), contents).unwrap();
        run_git(root, &["add", "--", "a.ts"]);
        run_git(root, &["commit", "-q", "-m", message]);
        run_git(root, &["rev-parse", "HEAD"])
    }

    #[test]
    fn history_drops_commits_that_only_edit_suppressions() {
        let repository = TempRepository::new();
        let root = repository.0.as_path();
        let first = commit_file(root, "let x = 0;\n", "one");
        let second = commit_file(root, "let x = 1;\n", "two");
        let third = commit_file(root, "let x = 1; // @ts-expect-error why\n", "three");
        let files = BTreeMap::from([("a.ts".to_string(), String::new())]);

        assert_eq!(
            suppression_only_edits(root, None, "200", &files),
            BTreeSet::from([(third, "a.ts".to_string())])
        );
        let history = captured_history_at(root, None, &files);
        let ids = history
            .commits
            .iter()
            .map(|commit| commit.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![second.as_str(), first.as_str()]);

        // Git prints the newest patches and then fails on a missing blob. A
        // failed log discounts nothing, even the suppression edit it printed.
        let blob = run_git(root, &["rev-parse", &format!("{first}:a.ts")]);
        let object = root.join(".git/objects").join(&blob[..2]).join(&blob[2..]);
        let mut permissions = fs::metadata(&object).unwrap().permissions();
        #[expect(
            clippy::permissions_set_readonly_false,
            reason = "test removes a loose object"
        )]
        permissions.set_readonly(false);
        fs::set_permissions(&object, permissions).unwrap();
        fs::remove_file(&object).unwrap();
        assert!(suppression_only_edits(root, None, "200", &files).is_empty());
    }

    #[test]
    fn migration_artifact_prefilter_and_recognizer_fail_closed_independently() {
        let registry = crate::policy::registry("rust-v1").unwrap();
        assert!(migration_artifact_candidate(
            Path::new("migrations/V1__create_accounts.sql"),
            &registry
        ));
        assert!(!migration_artifact_candidate(
            Path::new("docs/migration-notes.md"),
            &registry
        ));

        let files = BTreeMap::new();
        let manifests = BTreeMap::new();
        let implementations = Vec::new();
        let index = crate::migration::MigrationArtifactIndex::default();
        let recognizer = ArtifactRecognizer {
            rule_pack: "rust-v1",
            files: &files,
            manifests: &manifests,
            implementations: &implementations,
            index: &index,
        };
        assert!(!recognizer.recognizes("migrations/V1__create_accounts.sql"));
    }

    #[test]
    fn combined_file_budget_counts_sources_and_artifacts_at_the_exact_boundary() {
        assert!(!maximum_file_budget_reached(2, 2, 5));
        assert!(maximum_file_budget_reached(2, 3, 5));
        assert!(maximum_file_budget_reached(usize::MAX, 1, usize::MAX));
    }

    #[test]
    fn git_work_tree_probe_requires_both_success_and_the_exact_true_response() {
        assert!(reports_inside_work_tree(true, b"true\n"));
        assert!(!reports_inside_work_tree(true, b"false\n"));
        assert!(!reports_inside_work_tree(false, b"true\n"));
    }

    #[test]
    fn shallow_boundary_truncates_only_a_partial_history_window() {
        assert!(shallow_window_truncated(true, 0));
        assert!(shallow_window_truncated(true, HISTORY_WINDOW - 1));
        assert!(!shallow_window_truncated(true, HISTORY_WINDOW));
        assert!(!shallow_window_truncated(false, 0));
    }

    #[test]
    fn parse_history_log_fails_closed_for_partial_shallow_window_and_accepts_full_window() {
        let files = BTreeMap::from([
            ("a.rs".into(), "fn a() {}\n".into()),
            ("b.rs".into(), "fn b() {}\n".into()),
        ]);
        let make_log = |count: usize| -> String {
            (0..count)
                .map(|index| {
                    if index == 0 {
                        format!("\u{1e}commit-{index}\nb.rs\na.rs\na.rs\nuntracked.rs\n")
                    } else {
                        format!("\u{1e}commit-{index} parent-{index}\nuntracked.rs\n")
                    }
                })
                .collect()
        };

        let partial = parse_history_log(&make_log(HISTORY_WINDOW - 1), &files, true);
        assert!(!partial.available);
        assert!(partial.shallow);
        assert!(partial.commits.is_empty());

        let full_shallow = parse_history_log(&make_log(HISTORY_WINDOW), &files, true);
        assert!(full_shallow.available);
        assert!(!full_shallow.shallow);
        assert_eq!(full_shallow.commits.len(), 1);
        assert_eq!(full_shallow.commits[0].id, "commit-0");
        assert_eq!(full_shallow.commits[0].files, vec!["a.rs", "b.rs"]);
        assert!(full_shallow.commits[0].root);

        let non_shallow = parse_history_log(&make_log(1), &files, false);
        assert!(non_shallow.available);
        assert!(!non_shallow.shallow);
        assert_eq!(non_shallow.commits.len(), 1);

        let parented = parse_history_log(
            "\u{1e}child root\u{1f}fix: the bug\na.rs\n\u{1e}root\u{1f}\nb.rs\n",
            &files,
            false,
        );
        assert_eq!(parented.commits[0].id, "child");
        assert_eq!(parented.commits[0].subject, "fix: the bug");
        assert_eq!(parented.commits[1].subject, "");
        assert!(!parented.commits[0].root);
        assert_eq!(parented.commits[1].id, "root");
        assert!(parented.commits[1].root);
    }

    #[test]
    fn suppression_only_changes_leave_their_commits() {
        let commit = |id: &str, files: &[&str]| HistoryCommit {
            id: id.into(),
            root: false,
            subject: String::new(),
            files: files.iter().map(ToString::to_string).collect(),
        };
        let mut history = History {
            available: true,
            shallow: false,
            commits: vec![commit("one", &["a.ts", "b.ts"]), commit("two", &["a.ts"])],
        };
        let suppression_only = BTreeSet::from([
            ("one".to_string(), "b.ts".to_string()),
            ("two".to_string(), "a.ts".to_string()),
        ]);
        without_suppression_edits(&mut history, &suppression_only);
        assert_eq!(history.commits.len(), 1);
        assert_eq!(history.commits[0].id, "one");
        assert_eq!(history.commits[0].files, vec!["a.ts"]);
    }

    #[test]
    fn staged_index_requires_three_fields_and_stage_zero_independently() {
        let parsed = parse_index(b"100644 abc 0\tsrc/lib.rs\0").unwrap();
        assert_eq!(
            parsed.get("src/lib.rs"),
            Some(&("100644".into(), "abc".into()))
        );
        for malformed in [
            &b"100644 abc\tsrc/lib.rs\0"[..],
            &b"100644 abc 1\tsrc/lib.rs\0"[..],
        ] {
            assert_eq!(
                parse_index(malformed).unwrap_err(),
                "unmerged or malformed staged snapshot"
            );
        }
    }

    #[test]
    fn staged_paths_reject_backslashes_and_escaping_components_independently() {
        validate_staged_path("src/lib.rs", Path::new("src/lib.rs")).unwrap();
        for (name, path) in [
            ("src\\lib.rs", Path::new("src\\lib.rs")),
            ("../lib.rs", Path::new("../lib.rs")),
        ] {
            assert_eq!(
                validate_staged_path(name, path).unwrap_err(),
                "non-portable or escaping staged source path"
            );
        }
    }

    #[test]
    fn parse_tree_keeps_blobs_skips_submodules_and_rejects_malformed_or_duplicate_entries() {
        let parsed =
            parse_tree(b"100644 blob abc123\tsrc/lib.rs\x00160000 commit def456\tsubmod\0")
                .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed.get("src/lib.rs"),
            Some(&("100644".into(), "abc123".into()))
        );
        assert_eq!(
            parse_tree(b"100644 blob\tsrc/lib.rs\0").unwrap_err(),
            "malformed tree entry"
        );
        assert_eq!(
            parse_tree(b"100644 blob abc\tsrc/lib.rs\x00100644 blob def\tsrc/lib.rs\0")
                .unwrap_err(),
            "duplicate tree path"
        );
    }

    #[test]
    fn stable_index_validation_detects_a_real_index_change() {
        let repository = TempRepository::new();
        repository.stage("first\n");
        let initial = git(&repository.0, &["ls-files", "--stage", "-z"]).unwrap();
        validate_stable_index(&repository.0, &initial).unwrap();

        repository.stage("second\n");

        assert_eq!(
            validate_stable_index(&repository.0, &initial).unwrap_err(),
            "Git index changed during snapshot capture"
        );
    }

    proptest! {
        #[test]
        fn unresolved_migration_candidate_bytes_are_part_of_snapshot_identity(
            first in proptest::collection::vec(any::<u8>(), 0..128),
            second in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            prop_assume!(first != second);
            let digest_for = |body: Vec<u8>| {
                let snapshot = CapturedSnapshot {
                    files: BTreeMap::new(),
                    manifests: BTreeMap::new(),
                    directory_links: BTreeMap::new(),
                    implementations: vec![],
                    artifacts: BTreeMap::new(),
                    artifact_candidates: BTreeMap::from([("drizzle/0000_accounts.sql".into(), body)]),
                    history: History::default(),
                };
                snapshot_digest(&snapshot, "policy", b"selection")
            };
            prop_assert_ne!(digest_for(first), digest_for(second));
        }
    }
}
