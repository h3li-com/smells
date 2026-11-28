//! Picks and runs the project's own LCOV coverage tool for exact CRAP scores.
//!
//! Nothing here runs unless the user agrees to a prompt, because measuring coverage
//! runs the project's tests and may install a tool.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

/// One external command, shown to the user before it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    program: String,
    args: Vec<String>,
}

impl Step {
    pub(crate) fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }

    /// The command as the user would type it.
    pub(crate) fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Starts the command in `root` with all output appended to `log`.
    pub(crate) fn spawn(&self, root: &Path, log: &fs::File) -> std::io::Result<Child> {
        Command::new(&self.program)
            .args(&self.args)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log.try_clone()?)
            .spawn()
    }

    fn succeeds(&self, root: &Path) -> bool {
        Command::new(&self.program)
            .args(&self.args)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

/// How to measure coverage for one project.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CoveragePlan {
    /// The tool the user may be missing, such as `cargo-llvm-cov`.
    pub(crate) tool: &'static str,
    /// Install steps, empty when the tool is already available.
    pub(crate) install: Vec<Step>,
    pub(crate) run: Step,
    /// LCOV path the run writes, relative to the project root.
    pub(crate) output: PathBuf,
}

/// Picks the coverage tool for the policy's `language`, or, without a readable policy,
/// from the project's single language manifest. Returns `None` when the language is
/// ambiguous or no supported test runner is set up.
pub(crate) fn plan(root: &Path, language: Option<&str>) -> Option<CoveragePlan> {
    plan_with(root, language, &|step: &Step| step.succeeds(root))
}

fn plan_with(
    root: &Path,
    language: Option<&str>,
    available: &dyn Fn(&Step) -> bool,
) -> Option<CoveragePlan> {
    match language.or_else(|| manifest_language(root))? {
        "rust" => Some(rust_plan(available)),
        "python" => Some(python_plan(root, available)),
        "typescript" => typescript_plan(root),
        _ => None,
    }
}

/// A Rust crate packaged for Python with maturin has both manifests, so manifests
/// name a language only when exactly one kind is present.
fn manifest_language(root: &Path) -> Option<&'static str> {
    let has = |names: &[&str]| names.iter().any(|name| root.join(name).is_file());
    let rust = has(&["Cargo.toml"]);
    let python = has(&["pyproject.toml", "setup.py", "requirements.txt"]);
    let typescript = has(&["tsconfig.json", "package.json"]);
    match (rust, python, typescript) {
        (true, false, false) => Some("rust"),
        (false, true, false) => Some("python"),
        (false, false, true) => Some("typescript"),
        _ => None,
    }
}

fn rust_plan(available: &dyn Fn(&Step) -> bool) -> CoveragePlan {
    let probe = Step::new("cargo", &["llvm-cov", "--version"]);
    let install = if available(&probe) {
        Vec::new()
    } else {
        vec![
            Step::new("rustup", &["component", "add", "llvm-tools-preview"]),
            Step::new("cargo", &["install", "cargo-llvm-cov", "--locked"]),
        ]
    };
    CoveragePlan {
        tool: "cargo-llvm-cov",
        install,
        run: Step::new(
            "cargo",
            &["llvm-cov", "--lcov", "--output-path", "lcov.info"],
        ),
        output: PathBuf::from("lcov.info"),
    }
}

fn python_plan(root: &Path, available: &dyn Fn(&Step) -> bool) -> CoveragePlan {
    const REPORT: [&str; 2] = ["--cov", "--cov-report=lcov:lcov.info"];
    let output = PathBuf::from("lcov.info");
    let uv = Step::new("uv", &["--version"]);
    if root.join("uv.lock").is_file() && available(&uv) {
        // `uv run --with` adds the plugins for this run only, so nothing is installed.
        return CoveragePlan {
            tool: "pytest-cov",
            install: Vec::new(),
            run: Step::new(
                "uv",
                &[
                    "run",
                    "--with",
                    "pytest",
                    "--with",
                    "pytest-cov",
                    "pytest",
                    REPORT[0],
                    REPORT[1],
                ],
            ),
            output,
        };
    }
    let python = [".venv/bin/python", ".venv/Scripts/python.exe"]
        .into_iter()
        .map(|relative| root.join(relative))
        .find(|path| path.is_file())
        .map_or_else(|| "python3".to_string(), |path| path.display().to_string());
    let probe = Step::new(python.as_str(), &["-c", "import pytest, pytest_cov"]);
    let install = if available(&probe) {
        Vec::new()
    } else {
        vec![Step::new(
            python.as_str(),
            &["-m", "pip", "install", "pytest", "pytest-cov"],
        )]
    };
    CoveragePlan {
        tool: "pytest-cov",
        install,
        run: Step::new(python.as_str(), &["-m", "pytest", REPORT[0], REPORT[1]]),
        output,
    }
}

/// Package manager commands chosen from the lockfile.
struct PackageManager {
    add_dev: Step,
    exec: &'static str,
    exec_args: &'static [&'static str],
}

impl PackageManager {
    fn detect(root: &Path) -> Self {
        let (program, add, exec, exec_args): (&str, &[&str], _, &'static [&'static str]) =
            if root.join("pnpm-lock.yaml").is_file() {
                ("pnpm", &["add", "-D"], "pnpm", &["exec"])
            } else if root.join("yarn.lock").is_file() {
                ("yarn", &["add", "-D"], "yarn", &[])
            } else if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
                ("bun", &["add", "-d"], "bunx", &[])
            } else {
                ("npm", &["install", "-D"], "npx", &[])
            };
        Self {
            add_dev: Step::new(program, add),
            exec,
            exec_args,
        }
    }

    fn exec(&self, args: &[&str]) -> Step {
        let all: Vec<&str> = self.exec_args.iter().chain(args).copied().collect();
        Step::new(self.exec, &all)
    }
}

fn typescript_plan(root: &Path) -> Option<CoveragePlan> {
    let manager = PackageManager::detect(root);
    let output = PathBuf::from("coverage/lcov.info");
    if let Some(version) = installed_version(root, "vitest") {
        let has_provider = ["@vitest/coverage-v8", "@vitest/coverage-istanbul"]
            .iter()
            .any(|name| root.join("node_modules").join(name).is_dir());
        let install = if has_provider {
            Vec::new()
        } else {
            // The provider must match the installed vitest version exactly.
            let mut add = manager.add_dev.clone();
            add.args.push(format!("@vitest/coverage-v8@{version}"));
            vec![add]
        };
        return Some(CoveragePlan {
            tool: "@vitest/coverage-v8",
            install,
            run: manager.exec(&["vitest", "run", "--coverage", "--coverage.reporter=lcov"]),
            output,
        });
    }
    installed_version(root, "jest").map(|_| CoveragePlan {
        tool: "jest",
        install: Vec::new(),
        run: manager.exec(&["jest", "--coverage", "--coverageReporters=lcov"]),
        output,
    })
}

fn installed_version(root: &Path, package: &str) -> Option<String> {
    let manifest = root.join("node_modules").join(package).join("package.json");
    let text = fs::read_to_string(manifest).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("version")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "smells-coverage-tool-{name}-{}",
                std::process::id()
            ));
            if root.exists() {
                fs::remove_dir_all(&root).expect("stale test project is removable");
            }
            for (path, text) in files {
                let path = root.join(path);
                fs::create_dir_all(path.parent().expect("test path has a parent"))
                    .expect("test directory is creatable");
                fs::write(path, text).expect("test file is writable");
            }
            Self(root)
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn commands(plan: &CoveragePlan) -> (Vec<String>, String, &Path) {
        (
            plan.install.iter().map(Step::display).collect(),
            plan.run.display(),
            &plan.output,
        )
    }

    #[test]
    fn rust_installs_cargo_llvm_cov_only_when_missing() {
        let project = Project::new("rust", &[("Cargo.toml", "")]);
        let missing = plan_with(&project.0, None, &|_| false).expect("rust plan");
        assert_eq!(missing.tool, "cargo-llvm-cov");
        assert_eq!(
            commands(&missing),
            (
                vec![
                    "rustup component add llvm-tools-preview".to_string(),
                    "cargo install cargo-llvm-cov --locked".to_string(),
                ],
                "cargo llvm-cov --lcov --output-path lcov.info".to_string(),
                Path::new("lcov.info"),
            )
        );
        let present = plan_with(&project.0, None, &|step| {
            step.display() == "cargo llvm-cov --version"
        })
        .expect("rust plan");
        assert!(
            present.install.is_empty(),
            "installed tool needs no install"
        );
    }

    #[test]
    fn python_prefers_uv_then_the_project_virtualenv() {
        let uv = Project::new("uv", &[("pyproject.toml", ""), ("uv.lock", "")]);
        let with_uv =
            plan_with(&uv.0, None, &|step| step.display() == "uv --version").expect("plan");
        assert_eq!(
            commands(&with_uv),
            (
                Vec::new(),
                "uv run --with pytest --with pytest-cov pytest --cov --cov-report=lcov:lcov.info"
                    .to_string(),
                Path::new("lcov.info"),
            )
        );
        let without_uv = plan_with(&uv.0, None, &|_| false).expect("plan");
        assert_eq!(
            commands(&without_uv),
            (
                vec!["python3 -m pip install pytest pytest-cov".to_string()],
                "python3 -m pytest --cov --cov-report=lcov:lcov.info".to_string(),
                Path::new("lcov.info"),
            )
        );

        let venv = Project::new("venv", &[("setup.py", ""), (".venv/bin/python", "")]);
        let python = venv.0.join(".venv/bin/python").display().to_string();
        let ready = plan_with(&venv.0, None, &|step| {
            step.display() == format!("{python} -c import pytest, pytest_cov")
        })
        .expect("plan");
        assert_eq!(
            commands(&ready),
            (
                Vec::new(),
                format!("{python} -m pytest --cov --cov-report=lcov:lcov.info"),
                Path::new("lcov.info"),
            )
        );
    }

    #[test]
    fn vitest_installs_the_matching_provider_with_the_lockfile_manager() {
        let vitest = r#"{"name":"vitest","version":"3.2.4"}"#;
        let pnpm = Project::new(
            "pnpm",
            &[
                ("package.json", "{}"),
                ("pnpm-lock.yaml", ""),
                ("node_modules/vitest/package.json", vitest),
            ],
        );
        let plan = plan_with(&pnpm.0, None, &|_| false).expect("plan");
        assert_eq!(
            commands(&plan),
            (
                vec!["pnpm add -D @vitest/coverage-v8@3.2.4".to_string()],
                "pnpm exec vitest run --coverage --coverage.reporter=lcov".to_string(),
                Path::new("coverage/lcov.info"),
            )
        );

        let ready = Project::new(
            "vitest-ready",
            &[
                ("tsconfig.json", "{}"),
                ("node_modules/vitest/package.json", vitest),
                ("node_modules/@vitest/coverage-istanbul/package.json", "{}"),
            ],
        );
        let plan = plan_with(&ready.0, None, &|_| false).expect("plan");
        assert!(plan.install.is_empty(), "an installed provider is reused");
        assert_eq!(
            plan.run.display(),
            "npx vitest run --coverage --coverage.reporter=lcov"
        );
    }

    #[test]
    fn package_managers_follow_the_lockfile() {
        let cases = [
            (
                "yarn.lock",
                "yarn add -D",
                "yarn jest --coverage --coverageReporters=lcov",
            ),
            (
                "bun.lock",
                "bun add -d",
                "bunx jest --coverage --coverageReporters=lcov",
            ),
            (
                "bun.lockb",
                "bun add -d",
                "bunx jest --coverage --coverageReporters=lcov",
            ),
        ];
        for (lockfile, add, run) in cases {
            let project = Project::new(
                lockfile,
                &[
                    ("package.json", "{}"),
                    (lockfile, ""),
                    ("node_modules/jest/package.json", r#"{"version":"30.0.0"}"#),
                ],
            );
            let manager = PackageManager::detect(&project.0);
            assert_eq!(manager.add_dev.display(), add, "{lockfile}");
            let plan = plan_with(&project.0, None, &|_| false).expect("jest plan");
            assert_eq!(plan.tool, "jest");
            assert!(plan.install.is_empty(), "jest ships coverage");
            assert_eq!(plan.run.display(), run, "{lockfile}");
        }
    }

    #[test]
    fn plan_probes_the_real_environment() {
        let project = Project::new("probe", &[("Cargo.toml", "")]);
        let rust = plan(&project.0, None).expect("a Cargo.toml project has a plan");
        assert_eq!(
            rust.run.display(),
            "cargo llvm-cov --lcov --output-path lcov.info"
        );
        assert_eq!(plan(&Project::new("probe-none", &[]).0, None), None);
    }

    #[test]
    fn no_plan_without_a_single_language_or_test_runner() {
        let none = Project::new("none", &[]);
        assert_eq!(plan_with(&none.0, None, &|_| true), None);
        let mixed = Project::new("mixed", &[("Cargo.toml", ""), ("package.json", "{}")]);
        assert_eq!(plan_with(&mixed.0, None, &|_| true), None);
        let mixed = Project::new("mixed-py", &[("Cargo.toml", ""), ("pyproject.toml", "")]);
        assert_eq!(plan_with(&mixed.0, None, &|_| true), None);
        let mixed = Project::new(
            "mixed-ts",
            &[("requirements.txt", ""), ("tsconfig.json", "")],
        );
        assert_eq!(plan_with(&mixed.0, None, &|_| true), None);
        let bare = Project::new("bare-ts", &[("package.json", "{}")]);
        assert_eq!(plan_with(&bare.0, None, &|_| true), None);
        let broken = Project::new(
            "broken-ts",
            &[
                ("package.json", "{}"),
                ("node_modules/vitest/package.json", "not json"),
                ("node_modules/jest/package.json", r#"{"version":1}"#),
            ],
        );
        assert_eq!(plan_with(&broken.0, None, &|_| true), None);
    }

    #[test]
    fn the_policy_language_settles_a_project_with_several_manifests() {
        let maturin = Project::new("maturin", &[("Cargo.toml", ""), ("pyproject.toml", "")]);
        let rust = plan_with(&maturin.0, Some("rust"), &|_| true).expect("rust plan");
        assert_eq!(rust.tool, "cargo-llvm-cov");
        let python = plan_with(&maturin.0, Some("python"), &|_| true).expect("python plan");
        assert_eq!(python.tool, "pytest-cov");
        let typescript = Project::new(
            "policy-ts",
            &[
                ("Cargo.toml", ""),
                ("package.json", "{}"),
                ("node_modules/jest/package.json", r#"{"version":"29.0.0"}"#),
            ],
        );
        let jest = plan_with(&typescript.0, Some("typescript"), &|_| true).expect("jest plan");
        assert_eq!(jest.tool, "jest");
        assert_eq!(plan_with(&maturin.0, Some("java"), &|_| true), None);
    }

    #[cfg(unix)]
    #[test]
    fn steps_spawn_in_the_root_and_probe_exit_status() {
        let project = Project::new("spawn", &[("marker", "")]);
        assert!(Step::new("test", &["-f", "marker"]).succeeds(&project.0));
        assert!(!Step::new("test", &["-f", "absent"]).succeeds(&project.0));
        assert!(!Step::new("smells-no-such-program", &[]).succeeds(&project.0));
        let log_path = project.0.join("log");
        let log = fs::File::create(&log_path).expect("log is creatable");
        let status = Step::new("sh", &["-c", "echo out; echo err >&2"])
            .spawn(&project.0, &log)
            .expect("sh spawns")
            .wait()
            .expect("sh exits");
        assert!(status.success(), "sh succeeded");
        assert_eq!(fs::read_to_string(log_path).expect("log"), "out\nerr\n");
    }
}
