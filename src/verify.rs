//! Verification plans and result interpretation. Commands come from deterministic detection
//! (or are typed by the user); models may suggest commands but never execute them.

use std::sync::OnceLock;

use regex::Regex;

use crate::contracts::{Inventory, Passport, TestSummary};
use crate::util::sh_quote;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    pub ecosystem: String,
    pub image: String,
    pub install: Option<String>,
    pub syntax: Option<String>,
    pub test: Option<String>,
    pub env: Vec<(String, String)>,
    pub note: Option<String>,
}

const PY_SYNTAX_CHECK: &str = "import ast,os,sys\nbad=0\nfor r,ds,fs in os.walk('.'):\n  ds[:]=[d for d in ds if d not in ('.git','node_modules','__pycache__','.tox','venv','.venv')]\n  for f in fs:\n    if f.endswith('.py'):\n      p=os.path.join(r,f)\n      try: ast.parse(open(p,'rb').read(),p)\n      except SyntaxError as e:\n        bad+=1; print('SyntaxError: %s:%s: %s'%(p[2:],e.lineno,e.msg))\nprint('syntax check: %d file(s) failed to parse'%bad)\nsys.exit(1 if bad else 0)\n";

pub fn plan(inv: &Inventory, passport: &Passport, python_image: &str, node_image: &str) -> Plan {
    let primary = inv.languages.iter().find(|l| l.language != "Documentation" && l.language != "Config" && l.language != "Other").map(|l| l.language.as_str()).unwrap_or("");
    let has_pkg_json = inv.manifests.iter().any(|m| m.path == "package.json");
    if primary == "Python" || (inv.tests.framework == "pytest" || inv.tests.framework == "unittest") {
        let mut install = vec!["python -m pip install --quiet --no-cache-dir --target {DEPS} pytest".to_string()];
        let mut reqs: Vec<&str> = inv.manifests.iter().filter(|m| m.kind == "pip requirements").map(|m| m.path.as_str()).collect();
        reqs.sort();
        reqs.dedup();
        for r in &reqs {
            install[0].push_str(&format!(" -r {}", sh_quote(r)));
        }
        let setup_deps: Vec<String> = passport
            .dependencies
            .iter()
            .filter(|d| d.ecosystem == "pypi" && d.source_path.starts_with("setup.py"))
            .map(|d| sh_quote(&format!("{}{}", d.name, d.spec)))
            .collect();
        if !setup_deps.is_empty() {
            install[0].push(' ');
            install[0].push_str(&setup_deps.join(" "));
        }
        let note = if inv.tests.files.is_empty() { Some("No tests were detected; only the syntax check can run.".to_string()) } else { None };
        return Plan {
            ecosystem: "python".into(),
            image: python_image.into(),
            install: Some(install.remove(0)),
            syntax: Some(format!("python -c {}", sh_quote(PY_SYNTAX_CHECK))),
            test: if inv.tests.files.is_empty() { None } else { Some(pytest_command(&inv.tests.files)) },
            env: vec![
                ("PYTHONPATH".into(), "{WORK}:{WORK}/src:{DEPS}".into()),
                ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
                ("PYTHONUNBUFFERED".into(), "1".into()),
            ],
            note,
        };
    }
    if has_pkg_json {
        return Plan {
            ecosystem: "node".into(),
            image: node_image.into(),
            install: Some("npm install --ignore-scripts --no-audit --no-fund --loglevel=error".into()),
            syntax: None,
            test: if inv.tests.framework == "npm" { Some("npm test --silent".into()) } else { None },
            env: vec![],
            note: if inv.tests.framework == "npm" { None } else { Some("package.json has no test script.".into()) },
        };
    }
    Plan {
        ecosystem: "unsupported".into(),
        image: String::new(),
        install: None,
        syntax: None,
        test: None,
        env: vec![],
        note: Some(format!("Automated verification for '{primary}' projects is not implemented yet. Static analysis and agent review still apply.")),
    }
}

/// pytest only collects `test_*.py` / `*_test.py` by default; older projects often use `tests.py`.
/// When any test file falls outside the defaults, pass the Python test files explicitly.
fn pytest_command(test_files: &[String]) -> String {
    let base = "python -m pytest -q -p no:cacheprovider".to_string();
    let py: Vec<&String> = test_files.iter().filter(|f| f.ends_with(".py") && !f.ends_with("conftest.py") && !f.ends_with("__init__.py")).collect();
    let default_named = |f: &str| {
        let n = f.rsplit('/').next().unwrap_or(f);
        n.starts_with("test_") || n.ends_with("_test.py")
    };
    if py.iter().all(|f| default_named(f)) {
        return base;
    }
    let args: Vec<String> = py.iter().filter(|f| {
        let n = f.rsplit('/').next().unwrap_or(f);
        default_named(f) || n == "tests.py" || n == "test.py" || n.starts_with("test")
    }).take(50).map(|f| sh_quote(f)).collect();
    if args.is_empty() { base } else { format!("{base} {}", args.join(" ")) }
}

fn count_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(\d+) (passed|failed|errors?|skipped|passing|failing|pending)").unwrap())
}

fn jest_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"Tests:\s+(.*)\d+ total").unwrap())
}

/// Parse a test-runner summary (pytest, mocha, jest). Returns None when no summary is recognizable.
pub fn parse_summary(output: &str) -> Option<TestSummary> {
    let lines: Vec<&str> = output.lines().rev().take(40).collect();
    for line in &lines {
        if let Some(c) = jest_re().captures(line) {
            let mut s = TestSummary::default();
            for m in count_re().captures_iter(&c[1]) {
                let n: u32 = m[1].parse().unwrap_or(0);
                match &m[2] {
                    "passed" => s.passed = n,
                    "failed" => s.failed = n,
                    "skipped" => s.skipped = n,
                    _ => {}
                }
            }
            return Some(s);
        }
    }
    // pytest prints its summary on one line; mocha spreads passing/failing over several.
    let mut s = TestSummary::default();
    let mut found = false;
    for line in &lines {
        let is_pytest_summary = line.contains(" in ") && (line.contains("passed") || line.contains("failed") || line.contains("error") || line.contains("skipped"));
        let is_mocha = line.trim_start().chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && (line.contains("passing") || line.contains("failing") || line.contains("pending"));
        if !(is_pytest_summary || is_mocha) {
            continue;
        }
        for m in count_re().captures_iter(line) {
            let n: u32 = m[1].parse().unwrap_or(0);
            match &m[2] {
                "passed" | "passing" => s.passed = s.passed.max(n),
                "failed" | "failing" => s.failed = s.failed.max(n),
                "error" | "errors" => s.errors = s.errors.max(n),
                "skipped" | "pending" => s.skipped = s.skipped.max(n),
                _ => {}
            }
            found = true;
        }
        if is_pytest_summary && found {
            break;
        }
    }
    found.then_some(s)
}

/// Map an exit code + summary to a status. Compilation or a zero exit code alone is not "passed"
/// for a test step if the summary shows failures.
pub fn status_for(exit_code: Option<i32>, timed_out: bool, summary: Option<&TestSummary>, is_pytest: bool) -> (String, Option<String>) {
    if timed_out {
        return ("error".into(), Some("timed out".into()));
    }
    match exit_code {
        None => ("error".into(), Some("process ended without an exit code".into())),
        Some(125..=127) => ("error".into(), Some("the runner could not start the command".into())),
        Some(5) if is_pytest => ("error".into(), Some("pytest collected no tests".into())),
        Some(0) => match summary {
            Some(s) if s.failed > 0 || s.errors > 0 => ("failed".into(), None),
            _ => ("passed".into(), None),
        },
        Some(_) => ("failed".into(), None),
    }
}

/// True when `after` is worse than `before` (fewer passes or more failures/errors).
pub fn regressed(before: Option<&TestSummary>, after: Option<&TestSummary>, before_status: &str, after_status: &str) -> bool {
    match (before, after) {
        (Some(b), Some(a)) => a.passed < b.passed || a.failed + a.errors > b.failed + b.errors,
        _ => before_status == "passed" && after_status != "passed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pytest() {
        let out = "....F\n=========== 2 failed, 10 passed, 1 skipped, 3 errors in 0.52s ===========\n";
        let s = parse_summary(out).unwrap();
        assert_eq!(s, TestSummary { passed: 10, failed: 2, errors: 3, skipped: 1 });
        let s = parse_summary("12 passed in 0.10s\n").unwrap();
        assert_eq!(s.passed, 12);
        let s = parse_summary("!!! Interrupted: 2 errors during collection !!!\n2 errors in 0.31s\n").unwrap();
        assert_eq!(s.errors, 2);
    }

    #[test]
    fn parses_mocha_and_jest() {
        let s = parse_summary("  14 passing (30ms)\n  2 failing\n").unwrap();
        assert_eq!((s.passed, s.failed), (14, 2));
        let s = parse_summary("Tests:       1 failed, 2 skipped, 7 passed, 10 total\n").unwrap();
        assert_eq!((s.passed, s.failed, s.skipped), (7, 1, 2));
    }

    #[test]
    fn no_summary_is_none() {
        assert!(parse_summary("hello world\n").is_none());
    }

    #[test]
    fn status_rules() {
        assert_eq!(status_for(Some(0), false, None, true).0, "passed");
        assert_eq!(status_for(Some(1), false, None, true).0, "failed");
        assert_eq!(status_for(Some(5), false, None, true).0, "error");
        assert_eq!(status_for(None, true, None, true).0, "error");
        let s = TestSummary { passed: 1, failed: 1, errors: 0, skipped: 0 };
        assert_eq!(status_for(Some(0), false, Some(&s), true).0, "failed");
    }

    #[test]
    fn pytest_command_includes_nonstandard_test_files() {
        assert_eq!(pytest_command(&["tests/test_a.py".into()]), "python -m pytest -q -p no:cacheprovider");
        assert_eq!(pytest_command(&["tests.py".into()]), "python -m pytest -q -p no:cacheprovider 'tests.py'");
    }

    #[test]
    fn regression_detection() {
        let b = TestSummary { passed: 5, failed: 3, errors: 0, skipped: 0 };
        let a = TestSummary { passed: 8, failed: 0, errors: 0, skipped: 0 };
        assert!(!regressed(Some(&b), Some(&a), "failed", "passed"));
        assert!(regressed(Some(&a), Some(&b), "passed", "failed"));
    }
}
