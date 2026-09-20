use crate::policy::{Policy, Registry, SelectionOptions};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub struct Input {
    pub files: BTreeMap<String, String>,
    pub implementations: Vec<Implementation>,
    pub policy: Policy,
    pub digest: String,
    pub mode: &'static str,
}

#[derive(Clone, Debug)]
pub struct Implementation {
    pub id: String,
    pub root: Option<String>,
    pub ownership: &'static str,
    pub implementation_types: Vec<String>,
    pub runtime_types: Vec<String>,
    pub runtime_manifests: Vec<String>,
    pub source_files: Vec<String>,
}

pub struct CapturedInput {
    pub input: Input,
    pub registry: Registry,
}

pub fn digest(parts: &[&[u8]]) -> String {
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

fn runtime_manifest(path: &Path) -> Option<(&'static str, &'static str)> {
    match path.file_name().and_then(|value| value.to_str()) {
        Some("Cargo.toml") => Some(("rust_cargo_project", "rust")),
        Some("pyproject.toml") => Some(("python_project", "python")),
        Some("package.json") => Some(("javascript_typescript_package", "javascript_typescript")),
        _ => None,
    }
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
    for manifest in manifests.keys() {
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

fn walk(
    root: &Path,
    dir: &Path,
    policy: &Policy,
    registry: &Registry,
    files: &mut BTreeMap<String, String>,
    manifests: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("cannot enumerate source directory: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot enumerate source entry: {e}"))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        walk_entry(root, entry, policy, registry, files, manifests)?;
    }
    Ok(())
}

fn reject_symlink(path: &Path, relative: &Path, registry: &Registry) -> Result<(), String> {
    if source_file(path, &registry.language) {
        return Err(format!("symlink in source corpus: {}", relative.display()));
    }
    if runtime_manifest(path).is_some() {
        return Err(format!("symlink runtime manifest: {}", relative.display()));
    }
    Ok(())
}

fn capture_working_file(
    path: &Path,
    relative: &Path,
    policy: &Policy,
    files: &mut BTreeMap<String, String>,
    manifests: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let name = relative
        .to_str()
        .ok_or("source path is not UTF-8")?
        .to_string();
    if name.contains('\\') {
        return Err("backslash source paths are not supported in the portable corpus".into());
    }
    let source = text(
        fs::read(path).map_err(|e| format!("cannot read {name}: {e}"))?,
        &name,
    )?;
    if runtime_manifest(path).is_some() {
        manifests.insert(name, source);
    } else {
        if files.len() >= policy.limits.maximum_files {
            return Err("maximum_files budget exceeded".into());
        }
        files.insert(name, source);
    }
    Ok(())
}

fn walk_entry(
    root: &Path,
    entry: fs::DirEntry,
    policy: &Policy,
    registry: &Registry,
    files: &mut BTreeMap<String, String>,
    manifests: &mut BTreeMap<String, String>,
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
        return reject_symlink(&path, relative, registry);
    }
    if kind.is_dir() {
        return walk(root, &path, policy, registry, files, manifests);
    }
    if source_file(&path, &registry.language) || runtime_manifest(&path).is_some() {
        capture_working_file(&path, relative, policy, files, manifests)?;
    }
    Ok(())
}

fn snapshot_digest(
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    policy: &str,
    resolved_selection: &[u8],
) -> String {
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
    for (path, source) in files {
        parts.push(b"source");
        parts.push(path.as_bytes());
        parts.push(source.as_bytes());
    }
    digest(&parts)
}

pub fn working_tree(
    root: &Path,
    policy_path: &Path,
    selection: &SelectionOptions,
) -> Result<CapturedInput, String> {
    let root = fs::canonicalize(root).map_err(|e| format!("cannot open source root: {e}"))?;
    if !root.is_dir() {
        return Err("--path must be a source directory".into());
    }
    let policy_text = text(
        fs::read(policy_path).map_err(|e| format!("cannot read policy: {e}"))?,
        "policy",
    )?;
    let registry = crate::policy::registry_for_policy(&policy_text)?;
    let policy = crate::policy::parse_with_selection(&policy_text, &registry, selection)?;
    let mut files = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    walk(&root, &root, &policy, &registry, &mut files, &mut manifests)?;
    if files.is_empty() {
        return Err(format!(
            "source corpus contains no {} files",
            registry.language
        ));
    }
    Ok(CapturedInput {
        input: Input {
            digest: snapshot_digest(
                &files,
                &manifests,
                &policy_text,
                &serde_json::to_vec(&policy.resolved).expect("resolved selection serializes"),
            ),
            implementations: implementations(&files, &manifests),
            files,
            policy,
            mode: "working_tree",
        },
        registry,
    })
}

type IndexEntries = BTreeMap<String, (String, String)>;
type CapturedCorpus = (BTreeMap<String, String>, BTreeMap<String, String>);

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

fn repository_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root_text = text(git(&cwd, &["rev-parse", "--show-toplevel"])?, "Git root")?;
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

fn staged_corpus(
    root: &Path,
    entries: &IndexEntries,
    registry: &Registry,
    policy: &Policy,
) -> Result<CapturedCorpus, String> {
    let mut files = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for (name, (mode, oid)) in entries {
        let path = Path::new(name);
        if excluded(path, policy)
            || (!source_file(path, &registry.language) && runtime_manifest(path).is_none())
        {
            continue;
        }
        validate_staged_path(name, path)?;
        let is_manifest = runtime_manifest(path).is_some();
        let source = capture_staged_blob(root, name, mode, oid, is_manifest)?;
        if is_manifest {
            manifests.insert(name.clone(), source);
        } else {
            if files.len() >= policy.limits.maximum_files {
                return Err("maximum_files budget exceeded".into());
            }
            files.insert(name.clone(), source);
        }
    }
    Ok((files, manifests))
}

fn validate_stable_index(root: &Path, initial: &[u8]) -> Result<(), String> {
    if initial == git(root, &["ls-files", "--stage", "-z"])? {
        Ok(())
    } else {
        Err("Git index changed during snapshot capture".into())
    }
}

fn staged_input(
    files: BTreeMap<String, String>,
    manifests: BTreeMap<String, String>,
    policy_text: &str,
    policy: Policy,
    registry: Registry,
) -> Result<CapturedInput, String> {
    if files.is_empty() {
        return Err(format!(
            "staged corpus contains no {} files",
            registry.language
        ));
    }
    Ok(CapturedInput {
        input: Input {
            digest: snapshot_digest(
                &files,
                &manifests,
                policy_text,
                &serde_json::to_vec(&policy.resolved).expect("resolved selection serializes"),
            ),
            implementations: implementations(&files, &manifests),
            files,
            policy,
            mode: "staged_snapshot",
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

pub fn staged(
    policy_path: &Path,
    evidence_path: Option<&Path>,
    selection: &SelectionOptions,
) -> Result<(CapturedInput, Option<Vec<u8>>), String> {
    let policy_name = validate_staged_policy_path(policy_path)?;
    let root = repository_root()?;
    let initial = git(&root, &["ls-files", "--stage", "-z"])?;
    let entries = parse_index(&initial)?;
    let (policy_text, registry, policy) = staged_policy(&root, &entries, policy_name, selection)?;
    let (files, manifests) = staged_corpus(&root, &entries, &registry, &policy)?;
    let evidence = staged_evidence(&root, &entries, evidence_path)?;
    validate_stable_index(&root, &initial)?;
    Ok((
        staged_input(files, manifests, &policy_text, policy, registry)?,
        evidence,
    ))
}
