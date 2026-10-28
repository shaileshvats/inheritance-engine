//! Git operations. The original is kept as a read-only bare clone; the successor is a separate clone
//! on its own branch with every push URL disabled, so nothing the engine does can reach the source repository.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::contracts::{AuthorStat, CommitInfo, GitSummary};

pub const PUSH_DISABLED: &str = "PUSH-DISABLED-BY-INHERITANCE-ENGINE";
pub const SUCCESSOR_BRANCH: &str = "successor";

/// Hardening applied to every git invocation: no repository hooks, no symlink checkout,
/// no credential helpers, no `ext::` transport, no fsmonitor.
const HARDENING: &[&str] = &[
    "-c", "core.hooksPath=/dev/null",
    "-c", "core.symlinks=false",
    "-c", "core.fsmonitor=false",
    "-c", "credential.helper=",
    "-c", "protocol.ext.allow=never",
    "-c", "commit.gpgsign=false",
];

#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(data_dir: &Path, inheritance_id: &str) -> Self {
        Self { root: data_dir.join("inheritances").join(inheritance_id) }
    }
    pub fn original(&self) -> PathBuf {
        self.root.join("original.git")
    }
    pub fn successor(&self) -> PathBuf {
        self.root.join("successor")
    }
    /// Dependencies installed for native-mode verification (outside the successor tree).
    pub fn native_deps(&self) -> PathBuf {
        self.root.join("native-deps")
    }
    /// Throwaway HOME for native-mode runs.
    pub fn native_home(&self) -> PathBuf {
        self.root.join("native-home")
    }
}

fn base_command() -> Command {
    let mut c = Command::new("git");
    c.args(HARDENING)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ASKPASS", "echo")
        .env("LC_ALL", "C");
    c
}

pub fn git_in(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = base_command().arg("-C").arg(dir).args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_bare(git_dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = base_command().arg("--git-dir").arg(git_dir).args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub struct CloneResult {
    pub commit: String,
    pub branch: String,
}

/// Clone `url` into the layout: bare read-only original + successor working copy.
pub fn clone_inheritance(layout: &Layout, url: &str) -> anyhow::Result<CloneResult> {
    if !url.starts_with("https://github.com/") {
        anyhow::bail!("only public https://github.com/ repositories are supported in this version");
    }
    std::fs::create_dir_all(&layout.root)?;
    let original = layout.original();
    if !original.exists() {
        let tmp = layout.root.join("original.git.partial");
        let _ = std::fs::remove_dir_all(&tmp);
        let out = base_command().args(["clone", "--bare", "--quiet", "--no-recurse-submodules", url]).arg(&tmp).output()?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&tmp);
            anyhow::bail!("git clone failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        std::fs::rename(&tmp, &original)?;
    }
    let commit = git_bare(&original, &["rev-parse", "HEAD"])?.trim().to_string();
    let branch = git_bare(&original, &["symbolic-ref", "--short", "HEAD"]).map(|s| s.trim().to_string()).unwrap_or_else(|_| "HEAD".into());

    let successor = layout.successor();
    if !successor.exists() {
        let out = base_command().args(["clone", "--quiet", "--no-hardlinks", "--no-recurse-submodules"]).arg(&original).arg(&successor).output()?;
        if !out.status.success() {
            anyhow::bail!("successor clone failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        git_in(&successor, &["checkout", "-q", "-b", SUCCESSOR_BRANCH])?;
        git_in(&successor, &["remote", "set-url", "--push", "origin", PUSH_DISABLED])?;
        git_in(&successor, &["remote", "add", "upstream", url])?;
        git_in(&successor, &["remote", "set-url", "--push", "upstream", PUSH_DISABLED])?;
        git_in(&successor, &["config", "user.name", "Inheritance Engine"])?;
        git_in(&successor, &["config", "user.email", "successor@inheritance-engine.local"])?;
        git_in(&successor, &["config", "core.symlinks", "false"])?;
        git_in(&successor, &["config", "core.hooksPath", "/dev/null"])?;
        let exclude = successor.join(".git").join("info").join("exclude");
        std::fs::create_dir_all(exclude.parent().unwrap())?;
        std::fs::write(&exclude, "# added by Inheritance Engine: sandbox by-products\n__pycache__/\n*.pyc\n.pytest_cache/\nnode_modules/\n")?;
    }
    make_read_only(&original)?;
    Ok(CloneResult { commit, branch })
}

fn make_read_only(dir: &Path) -> anyhow::Result<()> {
    for entry in walkdir::WalkDir::new(dir).contents_first(true) {
        let entry = entry?;
        let mut perms = entry.metadata()?.permissions();
        if !perms.readonly() {
            perms.set_readonly(true);
            std::fs::set_permissions(entry.path(), perms)?;
        }
    }
    Ok(())
}

/// Confirm the original still points at the recorded commit and is still read-only.
pub fn check_original(layout: &Layout, branch: &str, commit: &str) -> (bool, Option<String>, Option<String>) {
    let original = layout.original();
    let head = match git_bare(&original, &["rev-parse", &format!("refs/heads/{branch}")]) {
        Ok(h) => h.trim().to_string(),
        Err(e) => return (false, None, Some(e.to_string())),
    };
    if head != commit {
        return (false, Some(head), Some("original branch moved away from the recorded commit".into()));
    }
    let writable = std::fs::metadata(original.join("HEAD")).map(|m| !m.permissions().readonly()).unwrap_or(true);
    if writable {
        return (false, Some(head), Some("original clone is no longer read-only".into()));
    }
    (true, Some(head), None)
}

pub fn head(dir: &Path) -> anyhow::Result<String> {
    Ok(git_in(dir, &["rev-parse", "HEAD"])?.trim().to_string())
}

pub fn worktree_diff(dir: &Path) -> anyhow::Result<String> {
    git_in(dir, &["diff", "--no-color", "--no-ext-diff"])
}

pub fn changed_files(dir: &Path) -> anyhow::Result<Vec<String>> {
    Ok(git_in(dir, &["diff", "--name-only"])?.lines().map(|s| s.to_string()).collect())
}

pub fn range_diff(dir: &Path, base: &str) -> anyhow::Result<String> {
    git_in(dir, &["diff", "--no-color", "--no-ext-diff", base, "HEAD"])
}

/// Discard uncommitted changes to tracked files and remove untracked, non-ignored files.
pub fn revert_worktree(dir: &Path) -> anyhow::Result<()> {
    git_in(dir, &["checkout", "-q", "--", "."])?;
    git_in(dir, &["clean", "-fdq"])?;
    Ok(())
}

pub fn commit_all(dir: &Path, message: &str) -> anyhow::Result<String> {
    git_in(dir, &["add", "-A"])?;
    git_in(dir, &["commit", "-q", "--no-verify", "-m", message])?;
    head(dir)
}

pub fn commits_ahead(dir: &Path, base: &str) -> usize {
    git_in(dir, &["rev-list", "--count", &format!("{base}..HEAD")]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

pub fn successor_commits(dir: &Path, base: &str) -> anyhow::Result<Vec<CommitInfo>> {
    let out = git_in(dir, &["log", "--format=%H%x1f%aI%x1f%an%x1f%s", &format!("{base}..HEAD")])?;
    Ok(parse_log(&out))
}

fn parse_log(out: &str) -> Vec<CommitInfo> {
    out.lines()
        .filter_map(|l| {
            let mut p = l.split('\x1f');
            Some(CommitInfo { sha: p.next()?.into(), date: p.next()?.into(), author: p.next()?.into(), subject: p.next().unwrap_or("").into() })
        })
        .collect()
}

/// Read a file as it exists in the original commit (never touching the successor).
pub fn show_original(layout: &Layout, commit: &str, path: &str) -> anyhow::Result<String> {
    git_bare(&layout.original(), &["show", &format!("{commit}:{path}")])
}

pub struct History {
    pub summary: GitSummary,
    /// path -> number of commits touching it
    pub churn: std::collections::HashMap<String, usize>,
}

pub fn history(dir: &Path, commit: &str) -> anyhow::Result<History> {
    let out = git_in(dir, &["log", "--format=%H%x1f%aI%x1f%an%x1f%s", "-n", "5000", commit])?;
    let commits = parse_log(&out);
    let mut authors: std::collections::HashMap<String, usize> = Default::default();
    for c in &commits {
        *authors.entry(c.author.clone()).or_default() += 1;
    }
    let mut authors: Vec<AuthorStat> = authors.into_iter().map(|(name, commits)| AuthorStat { name, commits }).collect();
    authors.sort_by(|a, b| b.commits.cmp(&a.commits).then(a.name.cmp(&b.name)));
    authors.truncate(12);

    let names = git_in(dir, &["log", "--format=", "--name-only", "-n", "2000", commit])?;
    let mut churn: std::collections::HashMap<String, usize> = Default::default();
    for l in names.lines().filter(|l| !l.trim().is_empty()) {
        *churn.entry(l.trim().to_string()).or_default() += 1;
    }
    Ok(History {
        summary: GitSummary {
            commits: commits.len(),
            first_commit_at: commits.last().map(|c| c.date.clone()),
            last_commit_at: commits.first().map(|c| c.date.clone()),
            authors,
            recent: commits.iter().take(15).cloned().collect(),
        },
        churn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo(dir: &Path) {
        git_in(dir, &["init", "-q", "-b", "main"]).unwrap();
        git_in(dir, &["config", "user.name", "T"]).unwrap();
        git_in(dir, &["config", "user.email", "t@example.com"]).unwrap();
        std::fs::write(dir.join("a.py"), "print('hi')\n").unwrap();
        git_in(dir, &["add", "."]).unwrap();
        git_in(dir, &["commit", "-q", "-m", "init"]).unwrap();
    }

    #[test]
    fn rejects_non_github_urls() {
        let d = tempfile::tempdir().unwrap();
        let layout = Layout::new(d.path(), "x");
        assert!(clone_inheritance(&layout, "file:///etc").is_err());
        assert!(clone_inheritance(&layout, "ext::sh -c touch% /tmp/pwned").is_err());
    }

    #[test]
    fn successor_changes_never_touch_original() {
        // Exercise the successor setup against a local source by calling the internals directly.
        let src = tempfile::tempdir().unwrap();
        init_repo(src.path());
        let data = tempfile::tempdir().unwrap();
        let layout = Layout::new(data.path(), "inh");
        std::fs::create_dir_all(&layout.root).unwrap();
        let out = base_command().args(["clone", "--bare", "--quiet"]).arg(src.path()).arg(layout.original()).output().unwrap();
        assert!(out.status.success());
        let out = base_command().args(["clone", "--quiet"]).arg(layout.original()).arg(layout.successor()).output().unwrap();
        assert!(out.status.success());
        git_in(&layout.successor(), &["checkout", "-q", "-b", SUCCESSOR_BRANCH]).unwrap();
        git_in(&layout.successor(), &["config", "user.name", "IE"]).unwrap();
        git_in(&layout.successor(), &["config", "user.email", "ie@example.com"]).unwrap();
        git_in(&layout.successor(), &["remote", "set-url", "--push", "origin", PUSH_DISABLED]).unwrap();
        let commit = git_bare(&layout.original(), &["rev-parse", "HEAD"]).unwrap().trim().to_string();
        make_read_only(&layout.original()).unwrap();

        std::fs::write(layout.successor().join("a.py"), "print('restored')\n").unwrap();
        let new = commit_all(&layout.successor(), "restore").unwrap();
        assert_ne!(new, commit);
        assert_eq!(commits_ahead(&layout.successor(), &commit), 1);

        // A push must fail because the push URL is disabled.
        assert!(git_in(&layout.successor(), &["push", "origin", "HEAD:main"]).is_err());

        let (ok, head, _) = check_original(&layout, "main", &commit);
        assert!(ok);
        assert_eq!(head.unwrap(), commit);
        assert_eq!(show_original(&layout, &commit, "a.py").unwrap(), "print('hi')\n");

        // Restore write permission so the tempdir can be cleaned up.
        for e in walkdir::WalkDir::new(layout.original()) {
            let e = e.unwrap();
            let mut p = e.metadata().unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            std::fs::set_permissions(e.path(), p).unwrap();
        }
    }
}
