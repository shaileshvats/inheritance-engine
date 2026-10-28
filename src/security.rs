//! Security primitives: workspace path containment, secret detection, and redaction.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::contracts::SecretHit;

/// Resolve a repository-relative path inside `root`, refusing anything that could escape it.
///
/// Rejects absolute paths, `..` components, paths into `.git`, and (after canonicalization) any path
/// whose real location is outside `root` — which also defeats symlinks pointing elsewhere.
pub fn safe_join(root: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() {
        anyhow::bail!("empty path");
    }
    if rel.contains('\0') {
        anyhow::bail!("path contains NUL");
    }
    let p = Path::new(rel);
    for c in p.components() {
        match c {
            Component::Normal(seg) => {
                if seg == ".git" {
                    anyhow::bail!("path '{rel}' points into .git");
                }
            }
            Component::CurDir => {}
            _ => anyhow::bail!("path '{rel}' is not a plain relative path"),
        }
    }
    let root_real = root.canonicalize()?;
    let joined = root_real.join(p);
    // The file may not exist yet; canonicalize the deepest existing ancestor.
    let mut probe = joined.clone();
    loop {
        if probe.exists() {
            let real = probe.canonicalize()?;
            if !real.starts_with(&root_real) {
                anyhow::bail!("path '{rel}' resolves outside the workspace");
            }
            break;
        }
        if !probe.pop() {
            break;
        }
    }
    Ok(joined)
}

struct SecretPattern {
    kind: &'static str,
    re: Regex,
}

fn patterns() -> &'static [SecretPattern] {
    static P: OnceLock<Vec<SecretPattern>> = OnceLock::new();
    P.get_or_init(|| {
        let mk = |kind, re: &str| SecretPattern { kind, re: Regex::new(re).expect("secret regex") };
        vec![
            mk("AWS access key", r"\b(AKIA|ASIA)[0-9A-Z]{16}\b"),
            mk("GitHub token", r"\bgh[pousr]_[A-Za-z0-9]{36,}\b"),
            mk("GitHub fine-grained token", r"\bgithub_pat_[A-Za-z0-9_]{40,}\b"),
            mk("Slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b"),
            mk("Google API key", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
            mk("OpenAI-style API key", r"\bsk-[A-Za-z0-9_\-]{20,}\b"),
            mk("Private key", r"-----BEGIN (RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----"),
            mk(
                "Hard-coded credential",
                r#"(?i)\b(api[_-]?key|secret[_-]?key|access[_-]?token|auth[_-]?token|password|passwd)\b\s*[:=]\s*['"][^'"\s]{8,}['"]"#,
            ),
        ]
    })
}

fn mask(s: &str) -> String {
    let keep: String = s.chars().take(4).collect();
    format!("{keep}…[redacted]")
}

/// Replace anything that looks like a secret with a masked form.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for p in patterns() {
        out = p.re.replace_all(&out, |c: &regex::Captures| mask(&c[0])).into_owned();
    }
    out
}

pub fn scan_secrets(path: &str, content: &str) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line.len() > 2000 {
            continue;
        }
        for p in patterns() {
            if let Some(m) = p.re.find(line) {
                hits.push(SecretHit { path: path.to_string(), line: i + 1, kind: p.kind.to_string(), redacted: mask(m.as_str()) });
            }
        }
    }
    hits
}

/// Environment variables that inherited code may see. Everything else is cleared.
pub const SANDBOX_ENV_ALLOWLIST: &[&str] = &["LANG", "LC_ALL", "TZ"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_git() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_join(dir.path(), "../etc/passwd").is_err());
        assert!(safe_join(dir.path(), "/etc/passwd").is_err());
        assert!(safe_join(dir.path(), ".git/config").is_err());
        assert!(safe_join(dir.path(), "a/../../b").is_err());
        assert!(safe_join(dir.path(), "src/main.py").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        assert!(safe_join(dir.path(), "link/secret.txt").is_err());
    }

    #[test]
    fn redacts_secrets() {
        let text = "token = ghp_abcdefghijklmnopqrstuvwxyz0123456789AB and AKIAABCDEFGHIJKLMNOP";
        let r = redact(text);
        assert!(!r.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789AB"));
        assert!(!r.contains("AKIAABCDEFGHIJKLMNOP"));
        assert!(r.contains("[redacted]"));
    }

    #[test]
    fn scans_hardcoded_password() {
        let hits = scan_secrets("cfg.py", "x = 1\npassword = 'hunter2hunter2'\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 2);
        assert!(!hits[0].redacted.contains("hunter2hunter2"));
    }
}
