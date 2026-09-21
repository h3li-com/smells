use crate::{
    input::Input,
    metrics::{self, Counter},
    policy::{Policy, Registry},
    report::{FindingRelations, Location, Report},
    similarity::exact_jaccard_pairs,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
#[cfg(unix)]
use std::{
    ffi::{CString, OsStr, OsString},
    os::unix::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawFd, FromRawFd},
    },
};
use tree_sitter::{Language, Node, Parser, Tree};

mod cache;
mod extract;
mod rules;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FunctionFact {
    symbol: String,
    location: Location,
    parameters: Vec<(String, String)>,
    lines: usize,
    comments: usize,
    tokens: Vec<String>,
    body_range: (usize, usize),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClassFact {
    symbol: String,
    location: Location,
    fields: usize,
    methods: usize,
    method_lines: usize,
    operations: usize,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    functions: Vec<FunctionFact>,
    classes: Vec<ClassFact>,
}

#[derive(Clone, Copy)]
struct RuleNeeds {
    functions: bool,
    parameters: bool,
    lines: bool,
    comments: bool,
    tokens: bool,
    classes: bool,
}

impl RuleNeeds {
    fn from_policy(policy: &Policy, language: &str) -> Self {
        let enabled = |suffix: &str| policy.enabled(&format!("{language}.{suffix}"));
        let parameters = enabled("function_arguments") || enabled("data_clumps");
        let lines = enabled("function_lines") || enabled("comment_share");
        let comments = enabled("comment_share");
        let tokens = enabled("duplicate_functions");
        let functions = parameters || lines || comments || tokens;
        let classes = [
            "class_fields",
            "class_methods",
            "class_method_lines",
            "data_class",
            "lazy_class",
        ]
        .into_iter()
        .any(enabled);
        Self {
            functions,
            parameters,
            lines,
            comments,
            tokens,
            classes,
        }
    }

    fn cache_bytes(self) -> [u8; 6] {
        [
            self.functions as u8,
            self.parameters as u8,
            self.lines as u8,
            self.comments as u8,
            self.tokens as u8,
            self.classes as u8,
        ]
    }
}

fn language(path: &str, registry: &Registry) -> Result<Language, String> {
    match registry.language.as_str() {
        "python" => Ok(tree_sitter_python::LANGUAGE.into()),
        "typescript" if Path::new(path).extension().is_some_and(|ext| ext == "tsx") => {
            Ok(tree_sitter_typescript::LANGUAGE_TSX.into())
        }
        "typescript" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        other => Err(format!("unsupported portable language: {other}")),
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceStats {
    syntax_nodes: usize,
    functions: usize,
    normalized_tokens: usize,
}

fn configured_parser(path: &str, registry: &Registry) -> Result<Parser, String> {
    let grammar = language(path, registry)?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|error| format!("cannot load grammar for {path}: {error}"))?;
    Ok(parser)
}

#[derive(Default)]
struct ParserPool {
    python: Option<Parser>,
    typescript: Option<Parser>,
    tsx: Option<Parser>,
}

impl ParserPool {
    fn parser(&mut self, path: &str, registry: &Registry) -> Result<&mut Parser, String> {
        let slot = match registry.language.as_str() {
            "python" => &mut self.python,
            "typescript" if Path::new(path).extension().is_some_and(|ext| ext == "tsx") => {
                &mut self.tsx
            }
            "typescript" => &mut self.typescript,
            other => return Err(format!("unsupported portable language: {other}")),
        };
        if slot.is_none() {
            *slot = Some(configured_parser(path, registry)?);
        }
        Ok(slot.as_mut().expect("parser initialized"))
    }
}

fn parse_source(
    path: &str,
    source: &str,
    registry: &Registry,
    parsers: &mut ParserPool,
) -> Result<Tree, String> {
    let parser = parsers.parser(path, registry)?;
    let tree = if registry.language == "typescript" {
        crate::typescript_compat::parse(parser, source, path)?
    } else {
        parser
            .parse(source, None)
            .ok_or_else(|| format!("parser cancelled for {path}"))?
    };
    if tree.root_node().has_error() {
        return Err(format!("parse error in {path}"));
    }
    Ok(tree)
}

pub fn check(input: &Input, registry: &Registry) -> Report {
    let mut report = Report::new(
        registry,
        &input.policy,
        input.mode,
        &input.implementations,
        &input.history,
    );
    report.begin_scan(input);
    rules::validate_required_rules(registry, input, &mut report);
    let needs = RuleNeeds::from_policy(&input.policy, &registry.language);
    let sources = input.files.iter().collect::<Vec<_>>();
    let analyses = sources
        .par_iter()
        .map_init(ParserPool::default, |parsers, (path, source)| {
            cache::cached_or_analyze_source(path, source, registry, needs, parsers)
        })
        .collect::<Vec<_>>();
    let mut facts = Facts::default();
    for mut analysis in analyses {
        facts.functions.append(&mut analysis.facts.functions);
        facts.classes.append(&mut analysis.facts.classes);
        report.extend_errors(analysis.errors.drain(..));
    }
    rules::source_metrics(&facts, registry, input, &mut report);
    rules::patterns(&facts, &input.policy, &registry.language, &mut report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_rejects_tampering_and_symlink_redirection() {
        let base =
            std::env::temp_dir().join(format!("smells-portable-cache-test-{}", std::process::id()));
        fs::create_dir(&base).unwrap();
        let root_path = base.join("cache");
        let root = cache::initialize_cache_root(&root_path).unwrap();
        let relative = Path::new("entry.json");
        let path = root_path.join(relative);
        let key = "a".repeat(64);
        let analysis = cache::SourceAnalysis::default();
        cache::store_analysis(&root, relative, &key, &analysis);
        assert!(cache::cached_analysis(&root, relative, &key).is_some());

        let mut entry: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        entry["analysis_sha256"] = json!("0".repeat(64));
        fs::write(&path, serde_json::to_vec(&entry).unwrap()).unwrap();
        assert!(cache::cached_analysis(&root, relative, &key).is_none());
        cache::store_analysis(&root, relative, &key, &analysis);
        assert!(cache::cached_analysis(&root, relative, &key).is_some());

        #[cfg(unix)]
        {
            let link = root_path.join("entry-link.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(cache::cached_analysis(&root, Path::new("entry-link.json"), &key).is_none());

            let actual = root_path.join("actual");
            fs::create_dir(&actual).unwrap();
            fs::copy(&path, actual.join(relative)).unwrap();
            std::os::unix::fs::symlink(&actual, root_path.join("linked-directory")).unwrap();
            let linked_entry = Path::new("linked-directory/entry.json");
            assert!(cache::cached_analysis(&root, linked_entry, &key).is_none());
            cache::store_analysis(&root, linked_entry, &key, &analysis);
            assert!(cache::cached_analysis(&root, linked_entry, &key).is_none());

            let moved = base.join("moved-cache");
            let attacker = base.join("attacker-cache");
            fs::create_dir(&attacker).unwrap();
            fs::rename(&root_path, &moved).unwrap();
            std::os::unix::fs::symlink(&attacker, &root_path).unwrap();
            fs::write(attacker.join(relative), b"not cache evidence").unwrap();
            assert!(cache::cached_analysis(&root, relative, &key).is_some());
            cache::store_analysis(&root, relative, &key, &analysis);
            assert!(cache::cached_analysis(&root, relative, &key).is_some());
            assert_eq!(
                fs::read(attacker.join(relative)).unwrap(),
                b"not cache evidence"
            );
            assert!(cache::initialize_cache_root(&root_path).is_none());
            fs::remove_file(&root_path).unwrap();
        }

        fs::remove_dir_all(base).unwrap();
    }
}
