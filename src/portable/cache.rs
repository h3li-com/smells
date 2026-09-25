use super::extract::extract_facts;
use super::*;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceAnalysis {
    pub(super) facts: Facts,
    pub(super) errors: Vec<SourceError>,
    stats: SourceStats,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheEntry {
    key: String,
    analysis_sha256: String,
    analysis: Box<serde_json::value::RawValue>,
}

#[derive(Serialize)]
struct CacheEntryRef<'a> {
    key: &'a str,
    analysis_sha256: String,
    analysis: &'a serde_json::value::RawValue,
}

pub(super) struct CacheRoot {
    #[cfg(not(unix))]
    path: PathBuf,
    #[cfg(unix)]
    directory: File,
}

fn cache_root() -> Option<&'static CacheRoot> {
    static ROOT: OnceLock<Option<CacheRoot>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let configured = std::env::var_os("SMELLS_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("smells-cache"));
        initialize_cache_root(&configured)
    })
    .as_ref()
}

fn fact_cache_key(path: &str, source: &str, registry: &Registry, needs: RuleNeeds) -> String {
    static IMPLEMENTATION: OnceLock<String> = OnceLock::new();
    let implementation = IMPLEMENTATION.get_or_init(|| {
        crate::input::digest(&[
            b"portable-facts-v1",
            include_bytes!("../portable.rs"),
            include_bytes!("../../Cargo.lock"),
        ])
    });
    crate::input::digest(&[
        implementation.as_bytes(),
        registry.language.as_bytes(),
        path.as_bytes(),
        &needs.cache_bytes(),
        source.as_bytes(),
    ])
}

fn fact_cache_relative_path(key: &str) -> PathBuf {
    PathBuf::from("portable-facts-v1")
        .join(&key[..2])
        .join(format!("{key}.json"))
}

fn analysis_digest(key: &str, bytes: &[u8]) -> String {
    crate::input::digest(&[b"portable-fact-payload-v1", key.as_bytes(), bytes])
}

#[cfg(unix)]
fn cache_components(relative: &Path) -> Option<Vec<CString>> {
    relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => CString::new(name.as_bytes()).ok(),
            _ => None,
        })
        .collect()
}

#[cfg(unix)]
fn open_at(parent: &File, name: &OsStr, flags: libc::c_int) -> Option<File> {
    let name = CString::new(name.as_bytes()).ok()?;
    // SAFETY: parent is an open directory descriptor, name is NUL-terminated,
    // and a successful openat returns a uniquely owned descriptor.
    let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if descriptor < 0 {
        None
    } else {
        // SAFETY: the successful openat call transferred ownership of descriptor.
        Some(unsafe { File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
fn open_absolute_directory(path: &Path) -> Option<File> {
    if !path.is_absolute() {
        return None;
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .ok()?;
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                directory = open_at(
                    &directory,
                    name,
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )?;
            }
            _ => return None,
        }
    }
    Some(directory)
}

#[cfg(unix)]
fn absolute_cache_path(configured: &Path) -> Option<PathBuf> {
    if configured.is_absolute() {
        Some(configured.to_path_buf())
    } else {
        Some(std::env::current_dir().ok()?.join(configured))
    }
}

#[cfg(unix)]
fn prepared_cache_parent(configured: &Path) -> Option<(File, OsString)> {
    let absolute = absolute_cache_path(configured)?;
    let parent = absolute.parent()?;
    fs::create_dir_all(parent).ok()?;
    let canonical_parent = parent.canonicalize().ok()?;
    Some((
        open_absolute_directory(&canonical_parent)?,
        absolute.file_name()?.to_os_string(),
    ))
}

#[cfg(unix)]
fn create_or_open_cache_root(parent: &File, name: &OsStr) -> Option<File> {
    let name_c = CString::new(name.as_bytes()).ok()?;
    // SAFETY: parent and name_c remain valid for the call.
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name_c.as_ptr(), 0o700) };
    if created != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        return None;
    }
    open_at(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )
}

#[cfg(unix)]
pub(super) fn initialize_cache_root(configured: &Path) -> Option<CacheRoot> {
    let (parent, name) = prepared_cache_parent(configured)?;
    Some(CacheRoot {
        directory: create_or_open_cache_root(&parent, &name)?,
    })
}

#[cfg(not(unix))]
fn absolute_cache_path(configured: &Path) -> Option<PathBuf> {
    if configured.is_absolute() {
        Some(configured.to_path_buf())
    } else {
        Some(std::env::current_dir().ok()?.join(configured))
    }
}

#[cfg(not(unix))]
pub(super) fn initialize_cache_root(configured: &Path) -> Option<CacheRoot> {
    let configured = absolute_cache_path(configured)?;
    fs::create_dir_all(&configured).ok()?;
    let path = configured.canonicalize().ok()?;
    fs::symlink_metadata(&path)
        .ok()?
        .file_type()
        .is_dir()
        .then_some(CacheRoot { path })
}

#[cfg(unix)]
fn open_cache_parent(root: &CacheRoot, relative: &Path, create: bool) -> Option<(File, CString)> {
    let mut components = cache_components(relative)?;
    let target = components.pop()?;
    let mut directory = root.directory.try_clone().ok()?;
    for component in components {
        if create {
            // SAFETY: directory and component remain valid for the duration of the call.
            let created =
                unsafe { libc::mkdirat(directory.as_raw_fd(), component.as_ptr(), 0o700) };
            if created != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
            {
                return None;
            }
        }
        directory = open_at(
            &directory,
            OsStr::from_bytes(component.as_bytes()),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
    }
    Some((directory, target))
}

#[cfg(unix)]
fn open_cached_file(root: &CacheRoot, relative: &Path) -> Option<File> {
    let (parent, target) = open_cache_parent(root, relative, false)?;
    open_at(
        &parent,
        OsStr::from_bytes(target.as_bytes()),
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )
    .filter(|file| file.metadata().is_ok_and(|metadata| metadata.is_file()))
}

#[cfg(not(unix))]
fn open_cached_file(root: &CacheRoot, relative: &Path) -> Option<File> {
    let path = root.path.join(relative);
    let metadata = fs::symlink_metadata(&path).ok()?;
    metadata
        .file_type()
        .is_file()
        .then(|| File::open(path).ok())?
}

pub(super) fn cached_analysis(
    root: &CacheRoot,
    relative: &Path,
    key: &str,
) -> Option<SourceAnalysis> {
    let entry: CacheEntry = serde_json::from_reader(open_cached_file(root, relative)?).ok()?;
    if entry.key != key
        || analysis_digest(key, entry.analysis.get().as_bytes()) != entry.analysis_sha256
    {
        return None;
    }
    serde_json::from_str(entry.analysis.get()).ok()
}

#[cfg(unix)]
fn write_cache_entry(root: &CacheRoot, relative: &Path, bytes: &[u8], nonce: usize) -> Option<()> {
    let (parent, target) = open_cache_parent(root, relative, true)?;
    let temporary = CString::new(format!(
        ".{}.{}.{}.tmp",
        target.to_string_lossy(),
        std::process::id(),
        nonce
    ))
    .ok()?;
    let mut file = open_at(
        &parent,
        OsStr::from_bytes(temporary.as_bytes()),
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )?;
    if file.write_all(bytes).is_err() {
        // SAFETY: parent and temporary remain valid for the duration of the call.
        unsafe { libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0) };
        return None;
    }
    drop(file);
    // SAFETY: both names are relative to the same open directory descriptor.
    let renamed = unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            temporary.as_ptr(),
            parent.as_raw_fd(),
            target.as_ptr(),
        )
    };
    if renamed != 0 {
        // SAFETY: parent and temporary remain valid for the duration of the call.
        unsafe { libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0) };
        return None;
    }
    Some(())
}

#[cfg(not(unix))]
fn write_cache_entry(root: &CacheRoot, relative: &Path, bytes: &[u8], nonce: usize) -> Option<()> {
    let path = root.path.join(relative);
    let parent = path.parent()?;
    fs::create_dir_all(parent).ok()?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()?.to_string_lossy(),
        std::process::id(),
        nonce
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| file.write_all(bytes))
        .ok()?;
    fs::rename(&temporary, path).ok()?;
    Some(())
}

pub(super) fn store_analysis(
    root: &CacheRoot,
    relative: &Path,
    key: &str,
    analysis: &SourceAnalysis,
) {
    static NEXT_TEMPORARY: AtomicUsize = AtomicUsize::new(0);
    let analysis = match serde_json::value::to_raw_value(analysis) {
        Ok(analysis) => analysis,
        Err(_) => return,
    };
    let analysis_sha256 = analysis_digest(key, analysis.get().as_bytes());
    let bytes = match serde_json::to_vec(&CacheEntryRef {
        key,
        analysis_sha256,
        analysis: &analysis,
    }) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let _ = write_cache_entry(
        root,
        relative,
        &bytes,
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed),
    );
}

pub(super) fn cached_or_analyze_source(
    path: &str,
    source: &str,
    registry: &Registry,
    needs: RuleNeeds,
    parsers: &mut ParserPool,
) -> SourceAnalysis {
    let key = fact_cache_key(path, source, registry, needs);
    let cache_relative = fact_cache_relative_path(&key);
    let cacheable = !needs.tokens;
    if cacheable
        && let Some(root) = cache_root()
        && let Some(analysis) = cached_analysis(root, &cache_relative, &key)
    {
        metrics::add(Counter::FactCacheHits, 1);
        record_analysis_stats(&analysis, false);
        return analysis;
    }
    metrics::add(Counter::FactCacheMisses, 1);
    let analysis = analyze_source(path, source, registry, needs, parsers);
    if cacheable && let Some(root) = cache_root() {
        store_analysis(root, &cache_relative, &key, &analysis);
    }
    record_analysis_stats(&analysis, true);
    analysis
}

fn record_analysis_stats(analysis: &SourceAnalysis, parsed: bool) {
    if parsed {
        metrics::add(Counter::SyntaxNodes, analysis.stats.syntax_nodes);
    }
    metrics::add(Counter::Functions, analysis.stats.functions);
    metrics::add(Counter::NormalizedTokens, analysis.stats.normalized_tokens);
}

fn analyze_source(
    path: &str,
    source: &str,
    registry: &Registry,
    needs: RuleNeeds,
    parsers: &mut ParserPool,
) -> SourceAnalysis {
    let tree = match parse_source(path, source, registry, parsers) {
        Ok(tree) => tree,
        Err(error) => {
            return SourceAnalysis {
                errors: vec![error],
                ..SourceAnalysis::default()
            };
        }
    };
    let (facts, stats) = extract_facts(&tree, path, source, registry, needs);
    SourceAnalysis {
        facts,
        errors: Vec::new(),
        stats,
    }
}
