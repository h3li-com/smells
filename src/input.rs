use crate::policy::{Policy, Registry};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub struct Input {
    pub files: BTreeMap<String, String>,
    pub policy: Policy,
    pub digest: String,
    pub mode: &'static str,
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

fn walk(
    root: &Path,
    dir: &Path,
    policy: &Policy,
    files: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("cannot enumerate source directory: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot enumerate source entry: {e}"))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "source path escapes root")?;
        if excluded(relative, policy) {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot inspect source entry: {e}"))?;
        if kind.is_symlink() {
            return Err(format!("symlink in source corpus: {}", relative.display()));
        }
        if kind.is_dir() {
            walk(root, &path, policy, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            if files.len() >= policy.limits.maximum_files {
                return Err("maximum_files budget exceeded".into());
            }
            let name = relative
                .to_str()
                .ok_or("source path is not UTF-8")?
                .to_string();
            if name.contains('\\') {
                return Err(
                    "backslash source paths are not supported in the portable corpus".into(),
                );
            }
            let source = text(
                fs::read(&path).map_err(|e| format!("cannot read {name}: {e}"))?,
                &name,
            )?;
            files.insert(name, source);
        }
    }
    Ok(())
}

fn snapshot_digest(files: &BTreeMap<String, String>, policy: &str) -> String {
    let mut parts = vec![policy.as_bytes()];
    for (path, source) in files {
        parts.push(path.as_bytes());
        parts.push(source.as_bytes());
    }
    digest(&parts)
}

pub fn working_tree(root: &Path, policy_path: &Path, registry: &Registry) -> Result<Input, String> {
    let root = fs::canonicalize(root).map_err(|e| format!("cannot open source root: {e}"))?;
    if !root.is_dir() {
        return Err("--path must be a source directory".into());
    }
    let policy_text = text(
        fs::read(policy_path).map_err(|e| format!("cannot read policy: {e}"))?,
        "policy",
    )?;
    let policy = crate::policy::parse(&policy_text, registry)?;
    let mut files = BTreeMap::new();
    walk(&root, &root, &policy, &mut files)?;
    if files.is_empty() {
        return Err("source corpus contains no Rust files".into());
    }
    Ok(Input {
        digest: snapshot_digest(&files, &policy_text),
        files,
        policy,
        mode: "working_tree",
    })
}

pub fn staged(policy_path: &Path, registry: &Registry) -> Result<Input, String> {
    if policy_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(
            "staged policy must be a repository-relative path without escaping components".into(),
        );
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root_text = text(git(&cwd, &["rev-parse", "--show-toplevel"])?, "Git root")?;
    let root = PathBuf::from(root_text.strip_suffix('\n').unwrap_or(&root_text));
    let initial = git(&root, &["ls-files", "--stage", "-z"])?;
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
    let name = policy_path.to_str().ok_or("policy path is not UTF-8")?;
    let (mode, oid) = entries
        .get(name)
        .ok_or("policy is not staged; refusing unstaged policy")?;
    if !matches!(mode.as_str(), "100644" | "100755") {
        return Err("staged policy is not a regular file".into());
    }
    let policy_text = text(git(&root, &["cat-file", "blob", oid])?, name)?;
    let policy = crate::policy::parse(&policy_text, registry)?;
    let mut files = BTreeMap::new();
    for (name, (mode, oid)) in &entries {
        let path = Path::new(name);
        if excluded(path, &policy) || path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        if name.contains('\\')
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("non-portable or escaping staged source path".into());
        }
        if !matches!(mode.as_str(), "100644" | "100755") {
            return Err(format!("staged source is not a regular file: {name}"));
        }
        if files.len() >= policy.limits.maximum_files {
            return Err("maximum_files budget exceeded".into());
        }
        files.insert(
            name.clone(),
            text(git(&root, &["cat-file", "blob", oid])?, name)?,
        );
    }
    if initial != git(&root, &["ls-files", "--stage", "-z"])? {
        return Err("Git index changed during snapshot capture".into());
    }
    if files.is_empty() {
        return Err("staged corpus contains no Rust files".into());
    }
    Ok(Input {
        digest: snapshot_digest(&files, &policy_text),
        files,
        policy,
        mode: "staged_snapshot",
    })
}
