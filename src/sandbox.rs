//! Execution adapters for inherited code. The control plane never executes repository code itself:
//! it either asks Docker to run it in a locked-down container, or (only when the user explicitly
//! chose native mode) spawns it with a cleared environment inside the successor directory.

use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::contracts::ExecMode;
use crate::gitops::Layout;

pub struct RunRequest<'a> {
    pub mode: ExecMode,
    pub layout: &'a Layout,
    pub inheritance_id: &'a str,
    pub image: &'a str,
    /// Shell command template; `{WORK}` and `{DEPS}` are substituted per mode.
    pub command: &'a str,
    pub env: &'a [(String, String)],
    /// Network access. Only dependency installation asks for it.
    pub network: bool,
    /// Run as root inside the container (needed to write the dependency volume). Never applies natively.
    pub as_root: bool,
    pub timeout: Duration,
    /// Mount this directory at /work instead of the successor (Docker only; used for the original export).
    pub workdir: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub exit_code: Option<i32>,
    pub output: String,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub environment: String,
    pub network: String,
    pub command: String,
}

pub async fn docker_version() -> Result<String, String> {
    let out = tokio::process::Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("docker CLI not available: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("docker daemon not reachable").to_string())
    }
}

pub fn deps_volume(inheritance_id: &str) -> String {
    format!("ie-deps-{}", inheritance_id.replace('_', "-"))
}

/// Make sure the image exists locally; pulling it is an explicit, logged network action.
pub async fn ensure_image(image: &str) -> anyhow::Result<bool> {
    let inspect = tokio::process::Command::new("docker").args(["image", "inspect", image]).stdout(Stdio::null()).stderr(Stdio::null()).status().await?;
    if inspect.success() {
        return Ok(false);
    }
    let pull = tokio::process::Command::new("docker").args(["pull", "--quiet", image]).output().await?;
    if !pull.status.success() {
        anyhow::bail!("docker pull {image} failed: {}", String::from_utf8_lossy(&pull.stderr).trim());
    }
    Ok(true)
}

pub fn render(template: &str, work: &str, deps: &str) -> String {
    template.replace("{WORK}", work).replace("{DEPS}", deps)
}

pub async fn run(req: RunRequest<'_>) -> anyhow::Result<RunOutcome> {
    match req.mode {
        ExecMode::Docker => run_docker(req).await,
        ExecMode::Native => run_native(req).await,
        ExecMode::None => anyhow::bail!("execution is disabled for this inheritance (static analysis only)"),
    }
}

fn docker_args(req: &RunRequest<'_>, name: &str) -> anyhow::Result<(Vec<String>, String)> {
    let successor = req.workdir.clone().unwrap_or_else(|| req.layout.successor()).canonicalize()?;
    let command = render(req.command, "/work", "/deps");
    let mut args: Vec<String> = vec![
        "run".into(), "--rm".into(), "--name".into(), name.into(),
        "--network".into(), if req.network { "bridge".into() } else { "none".into() },
        "--memory".into(), "1g".into(), "--memory-swap".into(), "1g".into(),
        "--cpus".into(), "2".into(), "--pids-limit".into(), "256".into(),
        "--cap-drop".into(), "ALL".into(), "--security-opt".into(), "no-new-privileges".into(),
        "--read-only".into(), "--tmpfs".into(), "/tmp:rw,exec,size=512m".into(),
        "-v".into(), format!("{}:/work:rw", successor.display()),
        "-v".into(), format!("{}:/deps:rw", deps_volume(req.inheritance_id)),
        "-w".into(), "/work".into(),
        "-e".into(), "HOME=/tmp".into(),
        "-e".into(), "PIP_DISABLE_PIP_VERSION_CHECK=1".into(),
        "-e".into(), "PIP_NO_INPUT=1".into(),
        "-e".into(), "npm_config_cache=/tmp/.npm".into(),
    ];
    if !req.as_root {
        args.push("--user".into());
        args.push("1000:1000".into());
    }
    for (k, v) in req.env {
        args.push("-e".into());
        args.push(format!("{k}={}", render(v, "/work", "/deps")));
    }
    args.push(req.image.into());
    args.push("sh".into());
    args.push("-c".into());
    args.push(format!("{command} 2>&1"));
    Ok((args, command))
}

async fn run_docker(req: RunRequest<'_>) -> anyhow::Result<RunOutcome> {
    let name = format!("ie-run-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let (args, command) = docker_args(&req, &name)?;
    let start = Instant::now();
    let child = tokio::process::Command::new("docker").args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn()?;
    let result = tokio::time::timeout(req.timeout, child.wait_with_output()).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let environment = format!("docker {} (network {}, 1 GiB RAM, 2 CPUs, read-only rootfs, caps dropped)", req.image, if req.network { "on" } else { "off" });
    let network = if req.network { "bridge".to_string() } else { "none".to_string() };
    match result {
        Ok(out) => {
            let out = out?;
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                text.push_str(&err);
            }
            Ok(RunOutcome { exit_code: out.status.code(), output: text, duration_ms, timed_out: false, environment, network, command })
        }
        Err(_) => {
            let _ = tokio::process::Command::new("docker").args(["kill", &name]).output().await;
            Ok(RunOutcome { exit_code: None, output: format!("timed out after {}s; container killed", req.timeout.as_secs()), duration_ms, timed_out: true, environment, network, command })
        }
    }
}

async fn run_native(req: RunRequest<'_>) -> anyhow::Result<RunOutcome> {
    if req.workdir.is_some() {
        anyhow::bail!("alternate working directories are only supported in Docker mode");
    }
    let successor = req.layout.successor().canonicalize()?;
    let deps = req.layout.native_deps();
    let home = req.layout.native_home();
    std::fs::create_dir_all(&deps)?;
    std::fs::create_dir_all(&home)?;
    let work_s = crate::util::sh_quote(&successor.display().to_string());
    let deps_s = crate::util::sh_quote(&deps.display().to_string());
    // Templates use bare {WORK}/{DEPS}; in native mode they become quoted absolute paths.
    let command = render(req.command, &work_s, &deps_s).replace("python ", "python3 ");
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(format!("{command} 2>&1"))
        .current_dir(&successor)
        .env_clear()
        .env("PATH", "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin")
        .env("HOME", &home)
        .env("TMPDIR", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in crate::security::SANDBOX_ENV_ALLOWLIST {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }
    for (k, v) in req.env {
        cmd.env(k, render(v, &successor.display().to_string(), &deps.display().to_string()));
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let start = Instant::now();
    let child = cmd.spawn()?;
    let pid = child.id();
    let result = tokio::time::timeout(req.timeout, child.wait_with_output()).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let environment = "native (cleared environment, workspace-scoped cwd; network NOT restricted)".to_string();
    match result {
        Ok(out) => {
            let out = out?;
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            Ok(RunOutcome { exit_code: out.status.code(), output: text, duration_ms, timed_out: false, environment, network: "host".into(), command })
        }
        Err(_) => {
            // Kill the whole process group directly (the child was started as its group leader), rather than spawning
            // `kill` and awaiting it: that extra child hung on GitHub's Ubuntu runners.
            #[cfg(unix)]
            if let Some(pid) = pid {
                // SAFETY: killpg only sends a signal; a group that has already exited yields ESRCH, which is ignored.
                unsafe {
                    libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                }
            }
            Ok(RunOutcome { exit_code: None, output: format!("timed out after {}s; process group killed", req.timeout.as_secs()), duration_ms, timed_out: true, environment, network: "host".into(), command })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_args_are_locked_down() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout { root: dir.path().to_path_buf() };
        std::fs::create_dir_all(layout.successor()).unwrap();
        let env = vec![("PYTHONPATH".to_string(), "{WORK}:{DEPS}".to_string())];
        let req = RunRequest {
            mode: ExecMode::Docker, layout: &layout, inheritance_id: "inh_1", image: "python:3.12-slim",
            command: "python -m pytest -q", env: &env, network: false, as_root: false, timeout: Duration::from_secs(5), workdir: None,
        };
        let (args, _) = docker_args(&req, "n").unwrap();
        let joined = args.join(" ");
        for must in ["--network none", "--cap-drop ALL", "--read-only", "no-new-privileges", "--pids-limit 256", "--memory 1g", "--user 1000:1000", "PYTHONPATH=/work:/deps"] {
            assert!(joined.contains(must), "missing {must} in {joined}");
        }
        // No host environment leaks: only explicitly listed -e flags.
        assert!(!joined.contains("GITHUB_TOKEN"));
    }

    #[tokio::test]
    async fn native_runs_with_cleared_environment() {
        std::env::set_var("IE_TEST_SECRET", "should-not-leak");
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout { root: dir.path().to_path_buf() };
        std::fs::create_dir_all(layout.successor()).unwrap();
        let req = RunRequest {
            mode: ExecMode::Native, layout: &layout, inheritance_id: "inh_1", image: "",
            command: "env; pwd", env: &[], network: false, as_root: false, timeout: Duration::from_secs(10), workdir: None,
        };
        let out = run(req).await.unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(!out.output.contains("should-not-leak"));
        assert!(out.output.contains(&layout.successor().canonicalize().unwrap().display().to_string()));
    }

    #[tokio::test]
    async fn native_timeout_kills() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout { root: dir.path().to_path_buf() };
        std::fs::create_dir_all(layout.successor()).unwrap();
        let req = RunRequest {
            mode: ExecMode::Native, layout: &layout, inheritance_id: "inh_1", image: "",
            command: "sleep 30", env: &[], network: false, as_root: false, timeout: Duration::from_millis(300), workdir: None,
        };
        let out = run(req).await.unwrap();
        assert!(out.timed_out);
        assert!(out.exit_code.is_none());
    }
}
