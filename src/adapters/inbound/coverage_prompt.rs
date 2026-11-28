//! Offers to measure coverage at an interactive terminal when no LCOV file exists.
//!
//! The offer never appears in CI, in hooks or for JSON output, and a failed run leaves
//! the scan on its zero-coverage CRAP estimates instead of guessing.

use std::{
    fmt::Write as _,
    fs,
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitStatus,
    thread,
    time::{Duration, Instant},
};

use crate::{
    adapters::outbound::coverage_tool::{CoveragePlan, Step},
    coverage,
};

const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const TICK: Duration = Duration::from_millis(100);

/// Whether a person can answer the prompt: stdin and stderr are terminals, `CI` is
/// unset or empty, and the output format is not machine-readable JSON.
pub(crate) fn should_offer(
    stdin_tty: bool,
    stderr_tty: bool,
    ci: Option<&str>,
    format: Option<&str>,
) -> bool {
    stdin_tty && stderr_tty && ci.is_none_or(str::is_empty) && format != Some("json")
}

/// Where the coverage prompt reads answers and draws progress.
pub(crate) struct Terminal<R, W> {
    interactive: bool,
    input: R,
    out: W,
}

impl Terminal<io::StdinLock<'static>, io::Stderr> {
    /// The process terminal, interactive only when [`should_offer`] allows it.
    pub(crate) fn detect(format: Option<&str>) -> Self {
        let ci = std::env::var("CI").ok();
        Self {
            interactive: should_offer(
                io::stdin().is_terminal(),
                io::stderr().is_terminal(),
                ci.as_deref(),
                format,
            ),
            input: io::stdin().lock(),
            out: io::stderr(),
        }
    }
}

impl<R: BufRead, W: Write> Terminal<R, W> {
    /// The explicit coverage file, else a conventional one under `root`, else the
    /// file an accepted offer writes.
    pub(crate) fn coverage(
        &mut self,
        explicit: Option<&Path>,
        root: &Path,
        plan: impl FnOnce(&Path) -> Option<CoveragePlan>,
    ) -> Option<PathBuf> {
        explicit
            .map(Path::to_path_buf)
            .or_else(|| coverage::discover(root))
            .or_else(|| self.offer(root, plan))
    }

    /// Asks to install the coverage tool if needed and run the tests, and returns the
    /// LCOV file the run wrote. Returns `None` without prompting when not interactive.
    pub(crate) fn offer(
        &mut self,
        root: &Path,
        plan: impl FnOnce(&Path) -> Option<CoveragePlan>,
    ) -> Option<PathBuf> {
        if !self.interactive {
            return None;
        }
        let plan = plan(root)?;
        let log = std::env::temp_dir().join(format!("smells-coverage-{}.log", std::process::id()));
        offer_plan(root, &plan, &log, &mut self.input, &mut self.out)
    }
}

fn offer_plan(
    root: &Path,
    plan: &CoveragePlan,
    log_path: &Path,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Option<PathBuf> {
    if !ask(plan, input, out).unwrap_or(false) {
        return None;
    }
    let log = match fs::File::create(log_path) {
        Ok(log) => log,
        Err(error) => {
            let _ = writeln!(out, "✗ cannot create {}: {error}", log_path.display());
            return None;
        }
    };
    let started = Instant::now();
    for step in plan.install.iter().chain([&plan.run]) {
        match run_with_spinner(step, root, &log, out) {
            Ok(status) if status.success() => {}
            Ok(status) => return failed(out, &step.display(), &status.to_string(), log_path),
            Err(error) => return failed(out, &step.display(), &error.to_string(), log_path),
        }
    }
    let output = root.join(&plan.output);
    if !output.is_file() {
        return failed(
            out,
            &plan.run.display(),
            &format!("it wrote no {}", plan.output.display()),
            log_path,
        );
    }
    let _ = writeln!(
        out,
        "✓ Coverage written to {} ({})",
        plan.output.display(),
        format_elapsed(started.elapsed())
    );
    Some(output)
}

fn ask(plan: &CoveragePlan, input: &mut impl BufRead, out: &mut impl Write) -> io::Result<bool> {
    out.write_all(question(plan).as_bytes())?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(confirmed(&answer))
}

fn question(plan: &CoveragePlan) -> String {
    let intro = if plan.install.is_empty() {
        "smells can measure it by running the project's tests:".to_string()
    } else {
        format!(
            "{} is not installed. smells can install it and run the project's tests:",
            plan.tool
        )
    };
    let install = plan.install.iter().fold(String::new(), |mut lines, step| {
        let _ = writeln!(lines, "  install: {}", step.display());
        lines
    });
    format!(
        "No LCOV coverage found, so CRAP scores assume 0% coverage.\n{intro}\n{install}  run:     {}\nRun now? [y/N] ",
        plan.run.display()
    )
}

fn confirmed(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn failed(out: &mut impl Write, command: &str, reason: &str, log_path: &Path) -> Option<PathBuf> {
    let _ = writeln!(
        out,
        "✗ `{command}` failed ({reason}); output is in {}. Continuing without coverage.",
        log_path.display()
    );
    None
}

/// Runs one step while redrawing a spinner and the elapsed time on one line.
fn run_with_spinner(
    step: &Step,
    root: &Path,
    log: &fs::File,
    out: &mut impl Write,
) -> io::Result<ExitStatus> {
    let mut child = step.spawn(root, log)?;
    let started = Instant::now();
    let command = step.display();
    for frame in FRAMES.iter().cycle() {
        if let Some(status) = child.try_wait()? {
            write!(out, "\r\x1b[2K")?;
            out.flush()?;
            return Ok(status);
        }
        write!(
            out,
            "\r{frame} {command} {}",
            format_elapsed(started.elapsed())
        )?;
        out.flush()?;
        thread::sleep(TICK);
    }
    unreachable!("cycling a non-empty frame list never ends")
}

fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_only_to_a_person_at_a_terminal() {
        assert!(should_offer(true, true, None, None), "terminal offers");
        assert!(
            should_offer(true, true, Some(""), Some("table")),
            "empty CI"
        );
        assert!(!should_offer(false, true, None, None), "piped stdin");
        assert!(!should_offer(true, false, None, None), "redirected stderr");
        assert!(!should_offer(true, true, Some("true"), None), "CI");
        assert!(!should_offer(true, true, None, Some("json")), "JSON output");
    }

    #[test]
    fn only_yes_confirms() {
        for answer in ["y\n", "Y", " yes \n", "YES"] {
            assert!(confirmed(answer), "{answer:?} confirms");
        }
        for answer in ["", "\n", "n", "no", "yep", "y es"] {
            assert!(!confirmed(answer), "{answer:?} declines");
        }
    }

    #[test]
    fn elapsed_time_reads_in_seconds_then_minutes() {
        assert_eq!(format_elapsed(Duration::from_millis(999)), "0s");
        assert_eq!(format_elapsed(Duration::from_secs(59)), "59s");
        assert_eq!(format_elapsed(Duration::from_secs(60)), "1m 00s");
        assert_eq!(format_elapsed(Duration::from_secs(125)), "2m 05s");
    }

    #[cfg(unix)]
    mod runs {
        use super::*;

        struct Project(PathBuf);

        impl Project {
            fn new(name: &str) -> Self {
                let root = std::env::temp_dir().join(format!(
                    "smells-coverage-prompt-{name}-{}",
                    std::process::id()
                ));
                if root.exists() {
                    fs::remove_dir_all(&root).expect("stale test project is removable");
                }
                fs::create_dir_all(&root).expect("test project is creatable");
                Self(root)
            }

            fn offer(&self, plan: &CoveragePlan, answer: &str) -> (Option<PathBuf>, String) {
                let mut out = Vec::new();
                let path = offer_plan(
                    &self.0,
                    plan,
                    &self.0.join("coverage.log"),
                    &mut answer.as_bytes(),
                    &mut out,
                );
                (
                    path,
                    String::from_utf8(out).expect("prompt output is UTF-8"),
                )
            }
        }

        impl Drop for Project {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn sh(script: &str) -> Step {
            Step::new("sh", &["-c", script])
        }

        fn plan(install: Vec<Step>, run: Step) -> CoveragePlan {
            CoveragePlan {
                tool: "fake-cov",
                install,
                run,
                output: PathBuf::from("lcov.info"),
            }
        }

        #[test]
        fn declining_runs_nothing() {
            let project = Project::new("decline");
            let (path, out) = project.offer(&plan(Vec::new(), sh("touch lcov.info")), "n\n");
            assert_eq!(path, None);
            assert!(
                !project.0.join("lcov.info").exists(),
                "declined run is skipped"
            );
            assert!(out.contains("run:     sh -c touch lcov.info"), "{out}");
            assert!(out.contains("running the project's tests"), "{out}");
            assert!(out.ends_with("Run now? [y/N] "), "{out}");
        }

        #[test]
        fn accepting_installs_then_runs_and_returns_the_lcov_file() {
            let project = Project::new("accept");
            let install = vec![sh("echo installed > tool")];
            let run = sh("sleep 0.25; test -f tool && echo SF:x > lcov.info");
            let (path, out) = project.offer(&plan(install, run), "y\n");
            assert_eq!(path, Some(project.0.join("lcov.info")));
            assert!(out.contains("fake-cov is not installed"), "{out}");
            assert!(
                out.contains("  install: sh -c echo installed > tool"),
                "{out}"
            );
            assert!(out.contains("⠋ sh -c sleep"), "spinner draws: {out}");
            assert!(out.contains("\r\x1b[2K"), "spinner clears its line: {out}");
            assert!(out.ends_with("s)\n"), "{out}");
            assert!(out.contains("✓ Coverage written to lcov.info ("), "{out}");
        }

        #[test]
        fn a_failed_step_stops_and_points_at_the_log() {
            let project = Project::new("fail");
            let install = vec![sh("echo broken >&2; exit 3")];
            let (path, out) = project.offer(&plan(install, sh("touch lcov.info")), "yes\n");
            assert_eq!(path, None);
            assert!(
                !project.0.join("lcov.info").exists(),
                "run after a failed install"
            );
            assert!(out.contains("exit status: 3"), "{out}");
            assert!(out.contains("Continuing without coverage."), "{out}");
            let log = fs::read_to_string(project.0.join("coverage.log")).expect("log");
            assert_eq!(log, "broken\n");
        }

        #[test]
        fn a_run_without_lcov_output_or_a_program_fails() {
            let project = Project::new("missing");
            let (path, out) = project.offer(&plan(Vec::new(), sh("true")), "y\n");
            assert_eq!(path, None);
            assert!(out.contains("it wrote no lcov.info"), "{out}");
            let absent = Step::new("smells-no-such-program", &[]);
            let (path, out) = project.offer(&plan(Vec::new(), absent), "y\n");
            assert_eq!(path, None);
            assert!(out.contains("`smells-no-such-program` failed ("), "{out}");
        }

        fn terminal(interactive: bool, answer: &str) -> Terminal<&[u8], Vec<u8>> {
            Terminal {
                interactive,
                input: answer.as_bytes(),
                out: Vec::new(),
            }
        }

        fn writes_lcov() -> impl FnOnce(&Path) -> Option<CoveragePlan> {
            |_| Some(plan(Vec::new(), sh("echo SF:x > lcov.info")))
        }

        #[test]
        fn coverage_prefers_explicit_then_found_then_an_accepted_offer() {
            let project = Project::new("resolve");
            let explicit = project.0.join("chosen.info");
            let mut asked = terminal(true, "y\n");
            assert_eq!(
                asked.coverage(Some(&explicit), &project.0, writes_lcov()),
                Some(explicit),
            );
            assert!(asked.out.is_empty(), "explicit coverage never prompts");

            let offered = asked.coverage(None, &project.0, writes_lcov());
            assert_eq!(offered, Some(project.0.join("lcov.info")));
            let out = String::from_utf8(asked.out).expect("prompt output is UTF-8");
            assert!(out.contains("✓ Coverage written to lcov.info"), "{out}");

            let mut found = terminal(true, "y\n");
            assert_eq!(
                found.coverage(None, &project.0, |_| panic!(
                    "found coverage skips the plan"
                )),
                Some(project.0.join("lcov.info")),
            );
            assert!(found.out.is_empty(), "found coverage never prompts");
        }

        #[test]
        fn offer_stays_silent_when_not_interactive_or_without_a_plan() {
            let project = Project::new("silent");
            let mut quiet = terminal(false, "y\n");
            assert_eq!(quiet.offer(&project.0, writes_lcov()), None);
            assert!(quiet.out.is_empty(), "no prompt outside a terminal");
            assert!(!project.0.join("lcov.info").exists(), "nothing ran");

            let mut unplanned = terminal(true, "y\n");
            assert_eq!(unplanned.offer(&project.0, |_| None), None);
            assert!(unplanned.out.is_empty(), "no prompt without a plan");
        }
    }
}
