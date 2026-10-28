//! Deterministic static analysis of a working tree. No model is involved here: these facts are
//! the grounded context the agents are given, and the evidence the UI shows.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::contracts::*;
use crate::security;

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "__pycache__", ".tox", ".venv", "venv", "env", ".mypy_cache", ".pytest_cache",
    ".eggs", "dist", "build", ".idea", ".vscode", "bower_components", "target",
];
const MAX_FILES: usize = 5000;
const MAX_ANALYZE_BYTES: u64 = 512 * 1024;

pub struct SourceFile {
    pub path: String,
    pub size: u64,
    pub lang: &'static str,
    pub text: Option<String>,
}

pub fn language_of(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    let name = lower.rsplit('/').next().unwrap_or("");
    match ext {
        "py" | "pyw" => "Python",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "rb" => "Ruby",
        "go" => "Go",
        "rs" => "Rust",
        "java" => "Java",
        "kt" => "Kotlin",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "C++",
        "cs" => "C#",
        "php" => "PHP",
        "sh" | "bash" => "Shell",
        "pl" | "pm" => "Perl",
        "lua" => "Lua",
        "swift" => "Swift",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "md" | "rst" | "txt" => "Documentation",
        "json" | "yml" | "yaml" | "toml" | "ini" | "cfg" => "Config",
        _ if name == "makefile" || name == "dockerfile" => "Build",
        _ => "Other",
    }
}

fn is_code(lang: &str) -> bool {
    !matches!(lang, "Documentation" | "Config" | "Other" | "Build" | "HTML" | "CSS")
}

pub fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or("");
    let in_test_dir = lower.split('/').any(|seg| matches!(seg, "test" | "tests" | "__tests__" | "spec" | "testing"));
    name.starts_with("test_") || name.ends_with("_test.py") || name == "tests.py" || name == "test.py" || name.contains(".test.") || name.contains(".spec.") || name == "conftest.py" || (in_test_dir && is_code(language_of(path)))
}

/// Walk the tree, collecting files (text loaded for analyzable sizes).
pub fn collect_files(root: &Path) -> Vec<SourceFile> {
    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(root).follow_links(false).sort_by_file_name().into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !(e.file_type().is_dir() && (SKIP_DIRS.contains(&name.as_ref()) || name.ends_with(".egg-info")))
    });
    for entry in walker.flatten() {
        if files.len() >= MAX_FILES {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let path = rel.to_string_lossy().replace('\\', "/");
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let lang = language_of(&path);
        let text = if size <= MAX_ANALYZE_BYTES {
            std::fs::read(entry.path()).ok().and_then(|b| {
                if b.iter().take(8192).any(|&c| c == 0) {
                    None
                } else {
                    Some(String::from_utf8_lossy(&b).into_owned())
                }
            })
        } else {
            None
        };
        files.push(SourceFile { path, size, lang, text });
    }
    files
}

fn build_tree(name: &str, files: &[SourceFile]) -> TreeNode {
    #[derive(Default)]
    struct Dir {
        dirs: BTreeMap<String, Dir>,
        files: Vec<(String, u64)>,
    }
    let mut root = Dir::default();
    for f in files {
        let parts: Vec<&str> = f.path.split('/').collect();
        let mut cur = &mut root;
        for seg in &parts[..parts.len() - 1] {
            cur = cur.dirs.entry(seg.to_string()).or_default();
        }
        cur.files.push((parts[parts.len() - 1].to_string(), f.size));
    }
    fn to_node(name: &str, path: &str, d: &Dir) -> TreeNode {
        let mut children = Vec::new();
        let mut size = 0;
        for (n, sub) in &d.dirs {
            let p = if path.is_empty() { n.clone() } else { format!("{path}/{n}") };
            let node = to_node(n, &p, sub);
            size += node.size;
            children.push(node);
        }
        for (n, s) in d.files.iter().take(300) {
            let p = if path.is_empty() { n.clone() } else { format!("{path}/{n}") };
            size += s;
            children.push(TreeNode { name: n.clone(), path: p, kind: "file".into(), size: *s, children: None });
        }
        TreeNode { name: name.to_string(), path: path.to_string(), kind: "dir".into(), size, children: Some(children) }
    }
    to_node(name, "", &root)
}

fn manifest_kind(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    Some(match name {
        "setup.py" => "setuptools (setup.py)",
        "setup.cfg" => "setuptools (setup.cfg)",
        "pyproject.toml" => "pyproject",
        "Pipfile" => "pipenv",
        "tox.ini" => "tox",
        "package.json" => "npm",
        "Cargo.toml" => "cargo",
        "go.mod" => "go modules",
        "Gemfile" => "bundler",
        "pom.xml" => "maven",
        "build.gradle" | "build.gradle.kts" => "gradle",
        "Makefile" | "makefile" => "make",
        "Dockerfile" => "docker",
        "composer.json" => "composer",
        n if n.starts_with("requirements") && n.ends_with(".txt") => "pip requirements",
        _ => return None,
    })
}

// ---------- import graph ----------

fn py_module_names(path: &str) -> Vec<String> {
    let Some(stem) = path.strip_suffix(".py") else { return vec![] };
    let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
    let full = stem.replace('/', ".");
    let mut names = vec![full.clone()];
    for prefix in ["src.", "lib.", "python."] {
        if let Some(s) = full.strip_prefix(prefix) {
            names.push(s.to_string());
        }
    }
    names
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

fn py_import_re() -> &'static (Regex, Regex) {
    static R: OnceLock<(Regex, Regex)> = OnceLock::new();
    R.get_or_init(|| (re(r"^\s*import\s+([\w\.]+(?:\s+as\s+\w+)?(?:\s*,\s*[\w\.]+(?:\s+as\s+\w+)?)*)"), re(r"^\s*from\s+(\.*)([\w\.]*)\s+import\s+(.+)")))
}

fn js_import_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r#"(?:require\(\s*['"](\.{1,2}/[^'"]+)['"]\s*\)|from\s+['"](\.{1,2}/[^'"]+)['"]|import\s+['"](\.{1,2}/[^'"]+)['"])"#))
}

fn resolve_longest(modules: &HashMap<String, String>, dotted: &str) -> Option<String> {
    let mut parts: Vec<&str> = dotted.split('.').filter(|s| !s.is_empty()).collect();
    while !parts.is_empty() {
        if let Some(p) = modules.get(&parts.join(".")) {
            return Some(p.clone());
        }
        parts.pop();
    }
    None
}

fn python_edges(files: &[SourceFile]) -> BTreeSet<GraphEdge> {
    let mut modules: HashMap<String, String> = HashMap::new();
    for f in files.iter().filter(|f| f.lang == "Python") {
        for n in py_module_names(&f.path) {
            modules.entry(n).or_insert_with(|| f.path.clone());
        }
    }
    let (imp, from) = py_import_re();
    let mut edges = BTreeSet::new();
    for f in files.iter().filter(|f| f.lang == "Python") {
        let Some(text) = &f.text else { continue };
        let own = py_module_names(&f.path).into_iter().next().unwrap_or_default();
        let is_pkg = f.path.ends_with("__init__.py");
        let package: Vec<&str> = {
            let mut p: Vec<&str> = own.split('.').collect();
            if !is_pkg {
                p.pop();
            }
            p
        };
        let mut add = |to: Option<String>| {
            if let Some(to) = to {
                if to != f.path {
                    edges.insert(GraphEdge { from: f.path.clone(), to });
                }
            }
        };
        for line in text.lines() {
            if let Some(c) = imp.captures(line) {
                for part in c[1].split(',') {
                    let name = part.split_whitespace().next().unwrap_or("");
                    add(resolve_longest(&modules, name));
                }
            } else if let Some(c) = from.captures(line) {
                let dots = c[1].len();
                let module = &c[2];
                let base = if dots == 0 {
                    module.to_string()
                } else {
                    let keep = package.len().saturating_sub(dots - 1);
                    let mut parts: Vec<&str> = package[..keep].to_vec();
                    if !module.is_empty() {
                        parts.push(module);
                    }
                    parts.join(".")
                };
                let names = c[3].replace(['(', ')', '\\'], " ");
                let mut any_sub = false;
                for n in names.split(',') {
                    let n = n.split_whitespace().next().unwrap_or("");
                    if n.is_empty() || n == "*" {
                        continue;
                    }
                    let sub = if base.is_empty() { n.to_string() } else { format!("{base}.{n}") };
                    if let Some(p) = modules.get(&sub) {
                        any_sub = true;
                        add(Some(p.clone()));
                    }
                }
                if !any_sub && !base.is_empty() {
                    add(resolve_longest(&modules, &base));
                }
            }
        }
    }
    edges
}

fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

fn js_edges(files: &[SourceFile]) -> BTreeSet<GraphEdge> {
    let known: BTreeSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut edges = BTreeSet::new();
    for f in files.iter().filter(|f| matches!(f.lang, "JavaScript" | "TypeScript")) {
        let Some(text) = &f.text else { continue };
        let dir = f.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for c in js_import_re().captures_iter(text) {
            let spec = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
            let base = normalize(&format!("{dir}/{spec}"));
            let candidates = [
                base.clone(),
                format!("{base}.js"), format!("{base}.ts"), format!("{base}.jsx"), format!("{base}.tsx"),
                format!("{base}.mjs"), format!("{base}.cjs"), format!("{base}/index.js"), format!("{base}/index.ts"),
            ];
            if let Some(hit) = candidates.iter().find(|c| known.contains(c.as_str())) {
                if hit != &f.path {
                    edges.insert(GraphEdge { from: f.path.clone(), to: hit.clone() });
                }
            }
        }
    }
    edges
}

// ---------- entry points ----------

fn entry_points(files: &[SourceFile], modules_by_path: &HashMap<String, String>) -> Vec<EntryPoint> {
    let main_guard = re(r#"^\s*if\s+__name__\s*==\s*['"]__main__['"]"#);
    let console = re(r#"['"]\s*[\w\-\.]+\s*=\s*([\w\.]+)\s*:\s*\w+"#);
    let mut eps = Vec::new();
    for f in files {
        let Some(text) = &f.text else { continue };
        let name = f.path.rsplit('/').next().unwrap_or("");
        if f.lang == "Python" {
            if name == "__main__.py" {
                eps.push(EntryPoint { path: f.path.clone(), kind: "python -m package".into(), detail: "package __main__ module".into() });
            }
            if let Some((i, _)) = text.lines().enumerate().find(|(_, l)| main_guard.is_match(l)) {
                if !is_test_path(&f.path) {
                    eps.push(EntryPoint { path: f.path.clone(), kind: "__main__ guard".into(), detail: format!("line {}", i + 1) });
                }
            }
        }
        if text.starts_with("#!") && is_code(f.lang) && !is_test_path(&f.path) {
            eps.push(EntryPoint { path: f.path.clone(), kind: "executable script".into(), detail: text.lines().next().unwrap_or("").chars().take(80).collect() });
        }
        if name == "setup.py" || name == "setup.cfg" || name == "pyproject.toml" {
            for c in console.captures_iter(text) {
                let module = &c[1];
                let target = modules_by_path.iter().find(|(_, m)| m.as_str() == module).map(|(p, _)| p.clone());
                if let Some(p) = target {
                    eps.push(EntryPoint { path: p, kind: "console script".into(), detail: format!("declared in {}", f.path) });
                }
            }
        }
        if name == "package.json" {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                let dir = f.path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                if let Some(m) = v.get("main").and_then(|m| m.as_str()) {
                    eps.push(EntryPoint { path: normalize(&format!("{dir}{m}")), kind: "package main".into(), detail: format!("declared in {}", f.path) });
                }
                match v.get("bin") {
                    Some(serde_json::Value::String(b)) => eps.push(EntryPoint { path: normalize(&format!("{dir}{b}")), kind: "npm bin".into(), detail: f.path.clone() }),
                    Some(serde_json::Value::Object(map)) => {
                        for (k, b) in map {
                            if let Some(b) = b.as_str() {
                                eps.push(EntryPoint { path: normalize(&format!("{dir}{b}")), kind: "npm bin".into(), detail: format!("command '{k}'") });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if f.lang == "Go" && text.contains("package main") && text.contains("func main(") {
            eps.push(EntryPoint { path: f.path.clone(), kind: "go main".into(), detail: "package main".into() });
        }
        if f.lang == "Rust" && (f.path.ends_with("src/main.rs") || f.path.contains("src/bin/")) {
            eps.push(EntryPoint { path: f.path.clone(), kind: "rust binary".into(), detail: String::new() });
        }
    }
    eps.sort_by(|a, b| a.path.cmp(&b.path).then(a.kind.cmp(&b.kind)));
    eps.dedup_by(|a, b| a.path == b.path && a.kind == b.kind);
    eps
}

// ---------- decay ----------

struct DecayRule {
    re: Regex,
    pattern: &'static str,
    detail: &'static str,
    removed_in: &'static str,
}

fn decay_rules() -> &'static [DecayRule] {
    static R: OnceLock<Vec<DecayRule>> = OnceLock::new();
    R.get_or_init(|| {
        let abc = "Mapping|MutableMapping|Sequence|MutableSequence|Set|MutableSet|Iterable|Iterator|Callable|Hashable|Sized|Container|Generator|KeysView|ItemsView|ValuesView|MappingView|Awaitable|Coroutine|AsyncIterable|AsyncIterator|ByteString";
        let mk = |re_s: &str, pattern, detail, removed_in| DecayRule { re: re(re_s), pattern, detail, removed_in };
        vec![
            mk(&format!(r"\bcollections\.({abc})\b"), "collections ABC alias", "Abstract base classes must be imported from collections.abc", "Python 3.10"),
            mk(&format!(r"^\s*from\s+collections\s+import\s+[^#]*\b({abc})\b"), "collections ABC import", "Abstract base classes must be imported from collections.abc", "Python 3.10"),
            mk(r"\.(assertEquals|assertNotEquals|assertAlmostEquals|assertNotAlmostEquals|assert_|failUnless|failIf|failUnlessEqual|failIfEqual|failUnlessRaises|failUnlessAlmostEqual|failIfAlmostEqual|assertRegexpMatches|assertNotRegexpMatches|assertRaisesRegexp|assertDictContainsSubset)\(", "deprecated unittest alias", "Deprecated unittest method aliases were removed", "Python 3.12"),
            mk(r"\.assertItemsEqual\(", "assertItemsEqual", "Python 2 only; use assertCountEqual", "Python 3.0"),
            mk(r"\binspect\.getargspec\(", "inspect.getargspec", "Use inspect.getfullargspec or inspect.signature", "Python 3.11"),
            mk(r"^\s*(import\s+imp\b|from\s+imp\s+import)", "imp module", "Use importlib", "Python 3.12"),
            mk(r"\btime\.clock\(", "time.clock", "Use time.perf_counter or time.process_time", "Python 3.8"),
            mk(r"@asyncio\.coroutine\b", "asyncio.coroutine", "Use async def", "Python 3.11"),
            mk(r"\basyncio\.async\(", "asyncio.async", "Use asyncio.ensure_future", "Python 3.7"),
            mk(r"^\s*(import\s+distutils|from\s+distutils)", "distutils", "Removed from the standard library; use setuptools or sysconfig", "Python 3.12"),
            mk(r"\bbase64\.(encodestring|decodestring)\(", "base64.encodestring", "Use encodebytes/decodebytes", "Python 3.9"),
            mk(r"\bcgi\.escape\(", "cgi.escape", "Use html.escape", "Python 3.8"),
            mk(r"\bfractions\.gcd\(", "fractions.gcd", "Use math.gcd", "Python 3.9"),
            mk(r"\bplatform\.(linux_distribution|dist)\(", "platform.linux_distribution", "Removed; use the distro package", "Python 3.8"),
            mk(r"\bos\.errno\b", "os.errno", "Import errno directly", "Python 3.7"),
            mk(r"\.isAlive\(\)", "Thread.isAlive", "Use is_alive()", "Python 3.9"),
            mk(r"\.getchildren\(\)", "Element.getchildren", "Use list(element)", "Python 3.9"),
            mk(r"\bxml\.etree\.cElementTree\b", "cElementTree", "Use xml.etree.ElementTree", "Python 3.9"),
            mk(r"\bjson\.loads?\([^)]*\bencoding\s*=", "json encoding argument", "The encoding argument was removed", "Python 3.9"),
            mk(r"^\s*(import\s+pkg_resources|from\s+pkg_resources)", "pkg_resources", "setuptools is not installed by default in modern environments; prefer importlib.metadata/resources", "Python 3.12 (setuptools no longer bundled)"),
            mk(r"^\s*print\s+[^\s(=]", "print statement", "Python 2 print statement", "Python 3.0"),
            mk(r"^\s*except\s+[\w\.]+(\s*\([^)]*\))?\s*,\s*\w+\s*:", "py2 except syntax", "Python 2 'except X, e' syntax", "Python 3.0"),
            mk(r"\.(iteritems|itervalues|iterkeys|has_key)\(", "py2 dict method", "Python 2 dict method (may be guarded by compatibility code)", "Python 3.0"),
            mk(r"\bsys\.maxint\b", "sys.maxint", "Use sys.maxsize", "Python 3.0"),
            mk(r"\bstring\.(letters|lowercase|uppercase|maketrans)\b", "string module constant", "Python 2 string module attribute", "Python 3.0"),
            mk(r"\blogging\.warn\(", "logging.warn", "Deprecated alias of logging.warning", "Python 3.13"),
            mk(r"\bnew Buffer\(", "new Buffer()", "Deprecated Node.js Buffer constructor; use Buffer.from/alloc", "Node.js 10 (deprecated)"),
        ]
    })
}

pub fn scan_decay(files: &[SourceFile]) -> Vec<DecayHit> {
    let mut hits = Vec::new();
    for f in files {
        let Some(text) = &f.text else { continue };
        let rules: Vec<&DecayRule> = decay_rules()
            .iter()
            .filter(|r| if r.pattern == "new Buffer()" { matches!(f.lang, "JavaScript" | "TypeScript") } else { f.lang == "Python" })
            .collect();
        if rules.is_empty() {
            continue;
        }
        for (i, line) in text.lines().enumerate() {
            for r in &rules {
                if let Some(m) = r.re.find(line) {
                    hits.push(DecayHit {
                        path: f.path.clone(),
                        line: i + 1,
                        pattern: r.pattern.to_string(),
                        detail: r.detail.to_string(),
                        removed_in: r.removed_in.to_string(),
                        text: line.trim().chars().take(200).collect(),
                        matched: m.as_str().trim().to_string(),
                    });
                }
            }
            if hits.len() >= 400 {
                return hits;
            }
        }
    }
    hits
}

// ---------- passport ----------

fn scan_signals(files: &[SourceFile], pattern: &Regex, only_code: bool) -> Vec<PassportSignal> {
    let mut out = Vec::new();
    for f in files {
        if only_code && !is_code(f.lang) {
            continue;
        }
        let Some(text) = &f.text else { continue };
        for (i, line) in text.lines().enumerate() {
            if line.len() < 2000 && pattern.is_match(line) {
                out.push(PassportSignal { path: f.path.clone(), line: i + 1, detail: security::redact(line.trim()).chars().take(160).collect() });
                if out.len() >= 60 {
                    return out;
                }
            }
        }
    }
    out
}

fn dependencies(files: &[SourceFile]) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let req_line = re(r"^\s*([A-Za-z0-9][A-Za-z0-9_\-\.]*)(\[[^\]]*\])?\s*([<>=!~;].*)?$");
    let quoted = re(r#"['"]([A-Za-z0-9][A-Za-z0-9_\-\.]*)(\[[^\]]*\])?\s*([<>=!~][^'"]*)?['"]"#);
    for f in files {
        let Some(text) = &f.text else { continue };
        let name = f.path.rsplit('/').next().unwrap_or("");
        if name.starts_with("requirements") && name.ends_with(".txt") {
            for l in text.lines() {
                let l = l.split('#').next().unwrap_or("").trim();
                if l.is_empty() || l.starts_with('-') {
                    continue;
                }
                if let Some(c) = req_line.captures(l) {
                    deps.push(Dependency { ecosystem: "pypi".into(), name: c[1].to_string(), spec: c.get(3).map(|m| m.as_str().trim().to_string()).unwrap_or_default(), source_path: f.path.clone() });
                }
            }
        } else if name == "setup.py" {
            for key in ["install_requires", "tests_require"] {
                if let Some(start) = text.find(key) {
                    let rest = &text[start..];
                    if let (Some(open), Some(close)) = (rest.find('['), rest.find(']')) {
                        if open < close {
                            for c in quoted.captures_iter(&rest[open..close]) {
                                deps.push(Dependency { ecosystem: "pypi".into(), name: c[1].to_string(), spec: c.get(3).map(|m| m.as_str().to_string()).unwrap_or_default(), source_path: format!("{} ({key})", f.path) });
                            }
                        }
                    }
                }
            }
        } else if name == "package.json" {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                for key in ["dependencies", "devDependencies"] {
                    if let Some(map) = v.get(key).and_then(|d| d.as_object()) {
                        for (k, spec) in map {
                            deps.push(Dependency { ecosystem: "npm".into(), name: k.clone(), spec: spec.as_str().unwrap_or("").to_string(), source_path: format!("{} ({key})", f.path) });
                        }
                    }
                }
            }
        }
    }
    deps
}

fn install_scripts(files: &[SourceFile]) -> Vec<PassportSignal> {
    let mut out = Vec::new();
    for f in files {
        let name = f.path.rsplit('/').next().unwrap_or("");
        let Some(text) = &f.text else { continue };
        if name == "setup.py" {
            let detail = if text.contains("cmdclass") { "setup.py with custom cmdclass: arbitrary code runs at install time" } else { "setup.py runs as Python code at install time" };
            out.push(PassportSignal { path: f.path.clone(), line: 1, detail: detail.into() });
        }
        if name == "package.json" {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
                    for hook in ["preinstall", "install", "postinstall", "prepare", "prepublish"] {
                        if let Some(cmd) = scripts.get(hook).and_then(|c| c.as_str()) {
                            out.push(PassportSignal { path: f.path.clone(), line: 1, detail: format!("npm {hook} script: {}", cmd.chars().take(120).collect::<String>()) });
                        }
                    }
                }
            }
        }
        if name == "Makefile" || name == "makefile" {
            out.push(PassportSignal { path: f.path.clone(), line: 1, detail: "Makefile targets run arbitrary shell commands".into() });
        }
    }
    out
}

fn passport(files: &[SourceFile], decay_free_unresolved: Vec<String>, provenance: Provenance) -> Passport {
    let network = re(r#"\b(requests\.(get|post|put|delete|patch|request|Session)|urllib(2|\.request)?\.(urlopen|Request)|http\.client|httplib|socket\.(socket|create_connection)|aiohttp|websocket|fetch\(|axios|XMLHttpRequest|https?\.(get|request)\(|smtplib|ftplib|paramiko)"#);
    let filesystem = re(r#"(open\([^)]*['"](w|a|wb|ab|w\+|r\+)['"]|os\.(remove|unlink|rmdir|makedirs|rename|chmod)\(|shutil\.(rmtree|move|copy)|fs\.(writeFile|unlink|rm|rmdir|appendFile)|os\.path\.expanduser|Path\.home\(\))"#);
    let process = re(r#"\b(subprocess\.|os\.(system|popen|exec\w*|spawn\w*)\(|child_process|execSync|spawnSync|commands\.getoutput)"#);
    let dynamic = re(r#"((^|[^\w\.])eval\(|(^|[^\w\.])exec\(|pickle\.loads?\(|marshal\.loads|yaml\.load\(|new Function\(|__import__\()"#);
    let mut secrets = Vec::new();
    for f in files {
        if let Some(t) = &f.text {
            secrets.extend(security::scan_secrets(&f.path, t));
        }
    }
    secrets.truncate(100);
    let mut unresolved = decay_free_unresolved;
    let deps = dependencies(files);
    if deps.iter().any(|d| d.spec.is_empty()) {
        unresolved.push("Some dependencies are unpinned; behaviour may differ from what the creator originally ran.".into());
    }
    let p = Passport {
        provenance,
        install_scripts: install_scripts(files),
        network_access: scan_signals(files, &network, true),
        filesystem_access: scan_signals(files, &filesystem, true),
        process_execution: scan_signals(files, &process, true),
        dynamic_code: scan_signals(files, &dynamic, true),
        secrets,
        dependencies: deps,
        unresolved,
    };
    let mut p = p;
    if !p.network_access.is_empty() {
        p.unresolved.push("Network-capable code is present. Verification runs with networking disabled, so its real network behaviour has not been observed.".into());
    }
    if !p.install_scripts.is_empty() {
        p.unresolved.push("Install-time scripts exist. The engine does not run them; dependency installation uses requirements files only (npm with --ignore-scripts).".into());
    }
    if !p.secrets.is_empty() {
        p.unresolved.push("Possible secrets were found in the source. They are redacted from agent prompts and traces; they were not rotated or used.".into());
    }
    p
}

/// Identify a license from a LICENSE/COPYING file when the forge reports none.
pub fn detect_license_file(root: &Path) -> Option<String> {
    let entries = std::fs::read_dir(root).ok()?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_uppercase();
        if !(name.starts_with("LICENSE") || name.starts_with("LICENCE") || name.starts_with("COPYING")) {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).ok()?;
        let t = text.to_ascii_lowercase();
        let id = if t.contains("apache license") && t.contains("version 2.0") {
            "Apache-2.0"
        } else if t.contains("permission is hereby granted, free of charge") {
            "MIT"
        } else if t.contains("gnu general public license") && t.contains("version 3") {
            "GPL-3.0"
        } else if t.contains("gnu general public license") && t.contains("version 2") {
            "GPL-2.0"
        } else if t.contains("gnu lesser general public license") {
            "LGPL"
        } else if t.contains("mozilla public license") {
            "MPL-2.0"
        } else if t.contains("redistribution and use in source and binary forms") {
            if t.contains("neither the name") { "BSD-3-Clause" } else { "BSD-2-Clause" }
        } else if t.contains("isc license") || t.contains("permission to use, copy, modify, and/or distribute") {
            "ISC"
        } else if t.contains("unlicense") || t.contains("public domain") {
            "Unlicense"
        } else {
            continue;
        };
        return Some(id.to_string());
    }
    None
}

// ---------- assembly ----------

pub struct InventoryInput<'a> {
    pub root: &'a Path,
    pub name: &'a str,
    pub commit: &'a str,
    pub provenance: Provenance,
}

pub fn build(input: InventoryInput) -> anyhow::Result<(Inventory, Passport)> {
    let files = collect_files(input.root);
    let history = crate::gitops::history(input.root, input.commit).unwrap_or(crate::gitops::History { summary: GitSummary::default(), churn: Default::default() });

    let mut lang_stats: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for f in &files {
        let e = lang_stats.entry(f.lang).or_default();
        e.0 += 1;
        e.1 += f.text.as_ref().map(|t| t.lines().count()).unwrap_or(0);
    }
    let mut languages: Vec<LanguageStat> = lang_stats.into_iter().map(|(l, (files, lines))| LanguageStat { language: l.into(), files, lines }).collect();
    languages.sort_by(|a, b| b.lines.cmp(&a.lines));

    let manifests: Vec<Manifest> = files.iter().filter_map(|f| manifest_kind(&f.path).map(|k| Manifest { path: f.path.clone(), kind: k.into() })).collect();

    let modules_by_path: HashMap<String, String> = files.iter().filter(|f| f.lang == "Python").filter_map(|f| py_module_names(&f.path).into_iter().next().map(|m| (f.path.clone(), m))).collect();
    let eps = entry_points(&files, &modules_by_path);

    let mut edges: BTreeSet<GraphEdge> = python_edges(&files);
    edges.extend(js_edges(&files));

    let test_files: Vec<String> = files.iter().filter(|f| is_test_path(&f.path)).map(|f| f.path.clone()).collect();
    let framework = detect_test_framework(&files, &test_files);

    let code_files: Vec<&SourceFile> = files.iter().filter(|f| is_code(f.lang) && f.lang != "Shell").collect();
    let mut fan_in: HashMap<&str, usize> = HashMap::new();
    let mut fan_out: HashMap<&str, usize> = HashMap::new();
    for e in &edges {
        *fan_in.entry(e.to.as_str()).or_default() += 1;
        *fan_out.entry(e.from.as_str()).or_default() += 1;
    }
    let entry_set: BTreeSet<&str> = eps.iter().map(|e| e.path.as_str()).collect();
    let nodes: Vec<GraphNode> = code_files
        .iter()
        .map(|f| GraphNode {
            id: f.path.clone(),
            path: f.path.clone(),
            lang: f.lang.into(),
            fan_in: *fan_in.get(f.path.as_str()).unwrap_or(&0),
            fan_out: *fan_out.get(f.path.as_str()).unwrap_or(&0),
            entry: entry_set.contains(f.path.as_str()),
            test: is_test_path(&f.path),
        })
        .collect();

    let readme = files.iter().find(|f| !f.path.contains('/') && f.path.to_ascii_lowercase().starts_with("readme"));
    let readme_text = readme.and_then(|r| r.text.clone()).unwrap_or_default().to_ascii_lowercase();
    let mother_candidates = score_mothers(&nodes, &code_files, &history.churn, &readme_text);

    let decay = scan_decay(&files);
    let mut unresolved = Vec::new();
    if input.provenance.license.is_none() {
        unresolved.push("No license metadata was detected. Absence of a license does not grant permission to redistribute.".into());
    }
    let passport = passport(&files, unresolved, input.provenance);

    let inventory = Inventory {
        files_total: files.len(),
        bytes_total: files.iter().map(|f| f.size).sum(),
        languages,
        tree: build_tree(input.name, &files),
        manifests,
        entry_points: eps,
        tests: TestInfo { framework, files: test_files },
        mother_candidates,
        graph: Graph { nodes, edges: edges.into_iter().collect() },
        git: history.summary,
        decay,
        readme_path: readme.map(|r| r.path.clone()),
    };
    Ok((inventory, passport))
}

fn detect_test_framework(files: &[SourceFile], test_files: &[String]) -> String {
    let has = |name: &str| files.iter().any(|f| f.path.rsplit('/').next() == Some(name));
    let mentions_pytest = files.iter().any(|f| {
        let n = f.path.rsplit('/').next().unwrap_or("");
        (n.starts_with("requirements") || matches!(n, "setup.py" | "setup.cfg" | "tox.ini" | "pyproject.toml")) && f.text.as_deref().unwrap_or("").contains("pytest")
    });
    let py_tests = test_files.iter().any(|t| t.ends_with(".py"));
    if py_tests && (has("conftest.py") || has("pytest.ini") || mentions_pytest) {
        return "pytest".into();
    }
    if py_tests {
        let unittest = files.iter().filter(|f| test_files.contains(&f.path)).any(|f| f.text.as_deref().unwrap_or("").contains("unittest"));
        return if unittest { "unittest".into() } else { "pytest".into() };
    }
    if let Some(pkg) = files.iter().find(|f| f.path == "package.json").and_then(|f| f.text.as_ref()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(pkg) {
            if let Some(t) = v.pointer("/scripts/test").and_then(|t| t.as_str()) {
                if !t.contains("no test specified") {
                    return "npm".into();
                }
            }
        }
    }
    "none".into()
}

fn score_mothers(nodes: &[GraphNode], code_files: &[&SourceFile], churn: &HashMap<String, usize>, readme: &str) -> Vec<MotherCandidate> {
    let max_in = nodes.iter().map(|n| n.fan_in).max().unwrap_or(0).max(1) as f64;
    let max_churn = nodes.iter().map(|n| *churn.get(&n.path).unwrap_or(&0)).max().unwrap_or(0).max(1) as f64;
    let lines_of = |p: &str| code_files.iter().find(|f| f.path == p).and_then(|f| f.text.as_ref()).map(|t| t.lines().count()).unwrap_or(0);
    let max_lines = nodes.iter().map(|n| lines_of(&n.path)).max().unwrap_or(0).max(1) as f64;
    let mut out: Vec<MotherCandidate> = nodes
        .iter()
        .filter(|n| !n.test && !n.path.ends_with("setup.py") && !n.path.contains("docs/") && !n.path.contains("example"))
        .map(|n| {
            let c = *churn.get(&n.path).unwrap_or(&0);
            let lines = lines_of(&n.path);
            let stem = n.path.rsplit('/').next().unwrap_or("").split('.').next().unwrap_or("").to_ascii_lowercase();
            let in_readme = stem.len() > 2 && stem != "__init__" && readme.contains(&stem);
            let score = 0.35 * (n.fan_in as f64 / max_in) + 0.25 * (c as f64 / max_churn) + 0.2 * if n.entry { 1.0 } else { 0.0 } + 0.1 * if in_readme { 1.0 } else { 0.0 } + 0.1 * ((lines as f64).sqrt() / max_lines.sqrt());
            let mut reasons = Vec::new();
            if n.fan_in > 0 {
                reasons.push(format!("imported by {} module{}", n.fan_in, if n.fan_in == 1 { "" } else { "s" }));
            }
            if c > 0 {
                reasons.push(format!("touched by {c} commit{}", if c == 1 { "" } else { "s" }));
            }
            if n.entry {
                reasons.push("detected entry point".into());
            }
            if in_readme {
                reasons.push(format!("'{stem}' is mentioned in the README"));
            }
            reasons.push(format!("{lines} lines"));
            MotherCandidate { path: n.path.clone(), score: (score * 100.0).round() / 100.0, reasons }
        })
        .collect();
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(a.path.cmp(&b.path)));
    out.truncate(10);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf(path: &str, text: &str) -> SourceFile {
        SourceFile { path: path.into(), size: text.len() as u64, lang: language_of(path), text: Some(text.into()) }
    }

    #[test]
    fn python_import_graph_resolves_relative_and_absolute() {
        let files = vec![
            sf("pkg/__init__.py", "from .core import thing\n"),
            sf("pkg/core.py", "import os\nfrom pkg import util\n"),
            sf("pkg/util.py", "x = 1\n"),
            sf("cli.py", "import pkg.core as c\nfrom pkg.util import x\n"),
            sf("tests/test_core.py", "from pkg import core\n"),
        ];
        let edges = python_edges(&files);
        let has = |a: &str, b: &str| edges.contains(&GraphEdge { from: a.into(), to: b.into() });
        assert!(has("pkg/__init__.py", "pkg/core.py"));
        assert!(has("pkg/core.py", "pkg/util.py"));
        assert!(has("cli.py", "pkg/core.py"));
        assert!(has("cli.py", "pkg/util.py"));
        assert!(has("tests/test_core.py", "pkg/core.py"));
        assert!(!edges.iter().any(|e| e.to == "os"));
    }

    #[test]
    fn js_import_graph_resolves_extensions() {
        let files = vec![sf("src/index.js", "const a = require('./lib/a');\nimport b from '../src/b';\n"), sf("src/lib/a.js", ""), sf("src/b.ts", "")];
        let edges = js_edges(&files);
        assert!(edges.contains(&GraphEdge { from: "src/index.js".into(), to: "src/lib/a.js".into() }));
        assert!(edges.contains(&GraphEdge { from: "src/index.js".into(), to: "src/b.ts".into() }));
    }

    #[test]
    fn decay_scan_finds_removed_apis() {
        let files = vec![sf(
            "m.py",
            "import collections\nclass A(collections.Mapping): pass\nfrom collections import OrderedDict, MutableMapping\nself.assertEquals(1, 1)\nprint 'hi'\nprint('ok')\n",
        )];
        let hits = scan_decay(&files);
        let patterns: Vec<&str> = hits.iter().map(|h| h.pattern.as_str()).collect();
        assert!(patterns.contains(&"collections ABC alias"));
        assert!(patterns.contains(&"collections ABC import"));
        assert!(patterns.contains(&"deprecated unittest alias"));
        assert!(patterns.contains(&"print statement"));
        assert_eq!(hits.iter().filter(|h| h.pattern == "print statement").count(), 1);
        assert_eq!(hits[0].line, 2);
    }

    #[test]
    fn largest_file_is_not_automatically_the_mother() {
        let nodes = vec![
            GraphNode { id: "big.py".into(), path: "big.py".into(), lang: "Python".into(), fan_in: 0, fan_out: 0, entry: false, test: false },
            GraphNode { id: "core.py".into(), path: "core.py".into(), lang: "Python".into(), fan_in: 5, fan_out: 0, entry: false, test: false },
        ];
        let big = sf("big.py", &"x = 1\n".repeat(2000));
        let core = sf("core.py", "def f(): pass\n");
        let files = vec![&big, &core];
        let churn: HashMap<String, usize> = [("core.py".to_string(), 10)].into_iter().collect();
        let m = score_mothers(&nodes, &files, &churn, "");
        assert_eq!(m[0].path, "core.py");
    }

    #[test]
    fn test_paths() {
        assert!(is_test_path("tests/test_x.py"));
        assert!(is_test_path("src/a.spec.ts"));
        assert!(!is_test_path("src/contest.py"));
        assert!(is_test_path("tests.py"));
    }
}
