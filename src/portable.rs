use crate::{
    core::model::Input,
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
struct SourceError {
    message: String,
    location: Location,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FunctionFact {
    symbol: String,
    owner: Option<String>,
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

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceStats {
    syntax_nodes: usize,
    functions: usize,
    normalized_tokens: usize,
}

fn configured_parser(path: &str, grammar: Language) -> Result<Parser, String> {
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
        let (slot, grammar) = match registry.language.as_str() {
            "python" => (&mut self.python, tree_sitter_python::LANGUAGE.into()),
            "typescript" if Path::new(path).extension().is_some_and(|ext| ext == "tsx") => {
                (&mut self.tsx, tree_sitter_typescript::LANGUAGE_TSX.into())
            }
            "typescript" => (
                &mut self.typescript,
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            ),
            other => return Err(format!("unsupported portable language: {other}")),
        };
        if slot.is_none() {
            *slot = Some(configured_parser(path, grammar)?);
        }
        Ok(slot.as_mut().expect("parser initialized"))
    }
}

fn parse_source(
    path: &str,
    source: &str,
    registry: &Registry,
    parsers: &mut ParserPool,
) -> Result<Tree, SourceError> {
    let at_start = |message| SourceError {
        message,
        location: Location {
            path: path.to_string(),
            line: 1,
            column: 1,
        },
    };
    let parser = parsers.parser(path, registry).map_err(at_start)?;
    let tree = if registry.language == "typescript" {
        crate::typescript_compat::parse(parser, source, path).map_err(at_start)?
    } else {
        parser
            .parse(source, None)
            .ok_or_else(|| at_start(format!("parser cancelled for {path}")))?
    };
    if tree.root_node().has_error() {
        let point = first_syntax_error(tree.root_node())
            .map(|node| node.start_position())
            .unwrap_or_else(|| tree.root_node().start_position());
        return Err(SourceError {
            message: format!("parse error in {path}"),
            location: Location {
                path: path.to_string(),
                line: point.row + 1,
                column: point.column + 1,
            },
        });
    }
    Ok(tree)
}

fn first_syntax_error(node: Node<'_>) -> Option<Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_syntax_error)
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
    for rule_id in crate::policy::required_unimplemented_rule_ids(registry, &input.policy) {
        report.error(format!("required detector not implemented: {rule_id}"));
    }
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
        for error in analysis.errors.drain(..) {
            report.error_at(error.message, error.location);
        }
    }
    rules::source_metrics(&facts, registry, input, &mut report);
    rules::patterns(&facts, &input.policy, &registry.language, &mut report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_bytes_encode_every_rule_need_independently() {
        for mask in 0_u8..64 {
            let needs = RuleNeeds {
                functions: mask & 1 != 0,
                parameters: mask & 2 != 0,
                lines: mask & 4 != 0,
                comments: mask & 8 != 0,
                tokens: mask & 16 != 0,
                classes: mask & 32 != 0,
            };
            assert_eq!(
                needs.cache_bytes(),
                [
                    u8::from(mask & 1 != 0),
                    u8::from(mask & 2 != 0),
                    u8::from(mask & 4 != 0),
                    u8::from(mask & 8 != 0),
                    u8::from(mask & 16 != 0),
                    u8::from(mask & 32 != 0),
                ]
            );
        }
    }

    #[test]
    fn parser_pool_selects_tsx_grammar_and_reports_real_error_locations() {
        let registry = crate::policy::registry("typescript-v1").unwrap();
        let mut parsers = ParserPool::default();
        let tsx = "export const View = () => <section>ok</section>;";
        assert!(parse_source("view.tsx", tsx, &registry, &mut parsers).is_ok());
        assert!(parse_source("view.ts", tsx, &registry, &mut parsers).is_err());

        let broken = "export const ok = 1;\nexport function broken( {";
        let error = parse_source("broken.ts", broken, &registry, &mut parsers).unwrap_err();
        assert_eq!(error.message, "parse error in broken.ts");
        assert_eq!(error.location.path, "broken.ts");
        assert_eq!(error.location.line, 2);
        assert_eq!(error.location.column, 1);
    }

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
        let analysis_bytes = serde_json::to_vec(&analysis).unwrap();
        assert_eq!(
            entry["analysis_sha256"],
            crate::input::digest(&[b"portable-fact-payload-v1", key.as_bytes(), &analysis_bytes,])
        );
        let reopened = cache::initialize_cache_root(&root_path).unwrap();
        assert!(cache::cached_analysis(&reopened, relative, &key).is_some());
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

            let blocked_entry = Path::new("blocked-entry");
            fs::create_dir(root_path.join(blocked_entry)).unwrap();
            cache::store_analysis(&root, blocked_entry, &key, &analysis);
            assert!(cache::cached_analysis(&root, blocked_entry, &key).is_none());
            assert!(fs::read_dir(&root_path).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".blocked-entry.")
            }));

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
