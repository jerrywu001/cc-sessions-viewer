use crate::types::{
    CliDiagnosisResult, CliHealthStatus, CliInstallation, CliUpgradeResult, CliVersionInfo,
};
use std::process::Command;
use std::time::Duration;

#[allow(dead_code)]
struct CliSpec {
    name: &'static str,
    binary: &'static str,
    npm_package: &'static str,
    /// Arguments for `brew upgrade` when installed via Homebrew Cask.
    brew_upgrade: Option<&'static str>,
    /// Built-in update subcommand (e.g. "claude update"), tried when the CLI
    /// wasn't installed via brew or npm.
    builtin_update: Option<&'static str>,
    /// Manifest URL template for non-npm CLIs (e.g. agy). The placeholder
    /// `{platform}` will be replaced at runtime with e.g. `darwin_arm64`.
    manifest_url: Option<&'static str>,
    /// Command that reports the latest version for self-managed CLIs.
    version_check_command: Option<&'static str>,
    /// Standalone install command for macOS / Linux (e.g. curl-based installer).
    install_unix: Option<&'static str>,
    /// Standalone install command for Windows (PowerShell, e.g. irm … | iex).
    install_windows: Option<&'static str>,
    /// A non-interactive update is safe to run from the viewer.
    background_upgrade: bool,
    /// Optional read-only health probe. Its output is never returned to the UI.
    health_check_command: Option<&'static str>,
}

const CLI_SPECS: &[CliSpec] = &[
    CliSpec {
        name: "claude",
        binary: "claude",
        npm_package: "@anthropic-ai/claude-code",
        brew_upgrade: Some("claude-code@latest"),
        builtin_update: Some("claude update"),
        manifest_url: None,
        version_check_command: None,
        install_unix: Some("curl -fsSL https://claude.ai/install.sh | bash"),
        install_windows: Some("irm https://claude.ai/install.ps1 | iex"),
        background_upgrade: true,
        health_check_command: None,
    },
    CliSpec {
        name: "codex",
        binary: "codex",
        npm_package: "@openai/codex",
        brew_upgrade: Some("--cask codex"),
        builtin_update: Some("codex update"),
        manifest_url: None,
        version_check_command: None,
        install_unix: Some("curl -fsSL https://chatgpt.com/codex/install.sh | sh"),
        install_windows: Some("irm https://chatgpt.com/codex/install.ps1 | iex"),
        background_upgrade: true,
        health_check_command: None,
    },
    CliSpec {
        name: "agy",
        binary: "agy",
        npm_package: "",
        brew_upgrade: None,
        builtin_update: Some("agy update"),
        manifest_url: Some(
            "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/{platform}.json",
        ),
        version_check_command: None,
        install_unix: Some("curl -fsSL https://antigravity.google/cli/install.sh | bash"),
        install_windows: Some("irm https://antigravity.google/cli/install.ps1 | iex"),
        background_upgrade: true,
        health_check_command: None,
    },
    CliSpec {
        name: "opencode",
        binary: "opencode",
        npm_package: "opencode-ai",
        brew_upgrade: Some("opencode"),
        builtin_update: Some("opencode upgrade"),
        manifest_url: None,
        version_check_command: None,
        install_unix: Some("curl -fsSL https://opencode.ai/install | bash"),
        install_windows: None,
        background_upgrade: true,
        health_check_command: None,
    },
    CliSpec {
        name: "grok",
        binary: "grok",
        npm_package: "",
        brew_upgrade: None,
        builtin_update: Some("grok update"),
        manifest_url: None,
        version_check_command: Some("grok update --check --json"),
        install_unix: Some("curl -fsSL https://x.ai/cli/install.sh | bash"),
        install_windows: Some("irm https://x.ai/cli/install.ps1 | iex"),
        background_upgrade: true,
        health_check_command: None,
    },
    CliSpec {
        name: "kimi",
        binary: "kimi",
        npm_package: "",
        brew_upgrade: None,
        builtin_update: None,
        manifest_url: None,
        version_check_command: None,
        install_unix: Some("curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash"),
        install_windows: Some("irm https://code.kimi.com/kimi-code/install.ps1 | iex"),
        // `kimi upgrade` can prompt, while Kimi manages its own default updates.
        background_upgrade: false,
        health_check_command: Some("kimi doctor"),
    },
];

fn find_spec(cli_name: &str) -> Result<&'static CliSpec, String> {
    CLI_SPECS
        .iter()
        .find(|s| s.name == cli_name)
        .ok_or_else(|| format!("unknown CLI: {cli_name}"))
}

// ---- shell helpers ----

#[cfg(unix)]
fn run_in_login_shell(cmd: &str) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    const DEFAULT_SHELL: &str = "/bin/zsh";
    #[cfg(not(target_os = "macos"))]
    const DEFAULT_SHELL: &str = "/bin/bash";

    let shell = std::env::var("SHELL").unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let out = Command::new(&shell)
        .args(["-l", "-i", "-c", cmd])
        .output()
        .map_err(|e| format!("shell exec: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("exit {}", out.status.code().unwrap_or(-1))
        } else {
            stderr
        })
    }
}

#[cfg(windows)]
fn run_in_login_shell(cmd: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    // -ExecutionPolicy Bypass（仅本进程）：npm/nvm 装的 claude/codex 是 .ps1 垫片，
    // Win 默认执行策略 Restricted 会拒跑它们，导致 `codex --version` 失败 → 误报"未安装"。
    // 前置 powershell_refresh_path()：从注册表重建完整 PATH，与 resume 路径同款解析，
    // 免得检测吃的是 GUI 进程继承的残缺 PATH、和 resume 实际会跑的命令不一致。
    let full_cmd = format!("{}; {cmd}", crate::agent_command::powershell_refresh_path());
    let out = Command::new(crate::agent_command::windows_powershell_exe())
        .args([
            "-NoLogo",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &full_cmd,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("powershell exec: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

// ---- version helpers ----

fn extract_version(output: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"(\d+\.\d+\.\d+)").ok()?;
    re.captures(output).map(|c| c[1].to_string())
}

fn get_installed_version(spec: &CliSpec) -> Option<String> {
    let out = run_in_login_shell(&format!("{} --version", spec.binary)).ok()?;
    extract_version(&out)
}

fn resolve_grok_binary() -> String {
    let managed = crate::util::home().join(".grok/bin/grok");
    if managed.is_file() {
        return managed.to_string_lossy().into_owned();
    }
    find_all_paths("grok")
        .into_iter()
        .next()
        .unwrap_or_else(|| "grok".to_string())
}

fn get_grok_installed_version() -> Option<String> {
    extract_version(&run_binary(&resolve_grok_binary(), &["--version"]).ok()?)
}

fn fetch_npm_latest(package: &str) -> Result<String, String> {
    let url = format!("https://registry.npmjs.org/{package}/latest");
    let try_once = || -> Result<String, String> {
        let resp: serde_json::Value = ureq::get(&url)
            .timeout(Duration::from_secs(10))
            .call()
            .map_err(|e| format!("npm registry: {e}"))?
            .into_json()
            .map_err(|e| format!("parse json: {e}"))?;
        resp.get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "missing version field".into())
    };
    match try_once() {
        Ok(v) => Ok(v),
        Err(_) => {
            std::thread::sleep(Duration::from_millis(500));
            try_once()
        }
    }
}

/// Fetch the latest version from a platform-specific manifest JSON endpoint.
/// The manifest is a JSON object with a `"version"` field.
fn fetch_manifest_latest(manifest_url_template: &str) -> Result<String, String> {
    let platform = detect_platform_key();
    let url = manifest_url_template.replace("{platform}", &platform);
    let try_once = || -> Result<String, String> {
        let resp: serde_json::Value = ureq::get(&url)
            .timeout(Duration::from_secs(10))
            .call()
            .map_err(|e| format!("manifest fetch: {e}"))?
            .into_json()
            .map_err(|e| format!("parse json: {e}"))?;
        resp.get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "missing version field".into())
    };
    match try_once() {
        Ok(v) => Ok(v),
        Err(_) => {
            std::thread::sleep(Duration::from_millis(500));
            try_once()
        }
    }
}

#[derive(Debug)]
struct CommandVersionInfo {
    latest_version: String,
    update_available: Option<bool>,
}

fn extract_command_latest_info(output: &str) -> Result<CommandVersionInfo, String> {
    // Prefer the explicit field in the raw command output. This avoids shell
    // wrappers or diagnostic JSON objects with an unrelated `version` field.
    if let Ok(re) =
        regex_lite::Regex::new(r#"[\"']latest(?:Version|_version)[\"']\s*:\s*[\"']([^\"']+)[\"']"#)
    {
        if let Some(caps) = re.captures(output) {
            let update_available =
                regex_lite::Regex::new(r#"[\"']updateAvailable[\"']\s*:\s*(true|false)"#)
                    .ok()
                    .and_then(|flag| flag.captures(output))
                    .and_then(|caps| caps.get(1).map(|v| v.as_str() == "true"));
            return Ok(CommandVersionInfo {
                latest_version: caps[1].to_string(),
                update_available,
            });
        }
    }
    let latest_from_value = |value: &serde_json::Value| {
        value
            .get("latestVersion")
            .or_else(|| value.get("latest_version"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };
    let generic_from_value = |value: &serde_json::Value| {
        value
            .get("version")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };
    let mut values = Vec::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
        values.push(value);
    }
    if let (Some(start), Some(end)) = (output.find('{'), output.rfind('}')) {
        if start < end {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&output[start..=end]) {
                values.push(value);
            }
        }
    }
    values.extend(
        output
            .lines()
            .rev()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()),
    );
    // A command may print unrelated JSON metadata before its update result.
    // Prefer the explicit latest-version fields across all parsed values before
    // falling back to a generic `version` field for other providers.
    for value in values.iter() {
        if let Some(version) = latest_from_value(value) {
            return Ok(CommandVersionInfo {
                latest_version: version,
                update_available: value.get("updateAvailable").and_then(|v| v.as_bool()),
            });
        }
    }
    for value in values.iter() {
        if let Some(version) = generic_from_value(value) {
            return Ok(CommandVersionInfo {
                latest_version: version,
                update_available: value.get("updateAvailable").and_then(|v| v.as_bool()),
            });
        }
    }
    Err("missing latest version field".into())
}

fn fetch_command_latest_info(command: &str) -> Result<CommandVersionInfo, String> {
    extract_command_latest_info(&run_in_login_shell(command)?)
}

fn run_binary(binary: &str, args: &[&str]) -> Result<String, String> {
    let home = crate::util::home();
    let grok_bin_dir = home.join(".grok/bin");
    let system_path = if cfg!(windows) {
        format!(
            "{};C:\\Windows\\System32;C:\\Windows",
            grok_bin_dir.display()
        )
    } else {
        format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", grok_bin_dir.display())
    };
    // Version probes must not inherit the GUI/terminal integration environment.
    // Grok's updater reads its own environment in addition to PATH, while pnpm
    // only resolves a binary from PATH.  Keeping this child deterministic also
    // prevents a shell wrapper or an injected CLI selector from changing the
    // update metadata returned to the UI.
    let mut cmd = Command::new(binary);
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    let out = cmd
        .args(args)
        .env_clear()
        .env("HOME", &home)
        .env("GROK_HOME", home.join(".grok"))
        .env("PATH", system_path)
        .env("TERM", "dumb")
        .env("LANG", "C.UTF-8")
        .current_dir(&home)
        .output()
        .map_err(|e| format!("binary exec: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("exit {}", out.status.code().unwrap_or(-1))
        } else {
            stderr
        })
    }
}

fn fetch_grok_latest_info() -> Result<CommandVersionInfo, String> {
    // Grok can be present more than once in PATH. Resolve the managed
    // executable directly so shell aliases or startup scripts cannot alter
    // the update-check JSON.
    let binary = resolve_grok_binary();
    extract_command_latest_info(&run_binary(&binary, &["update", "--check", "--json"])?)
}

/// Return a platform key such as `darwin_arm64`, `darwin_amd64`, `linux_amd64`
/// etc., matching the manifest filename convention used by the agy auto-updater.
fn detect_platform_key() -> String {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    format!("{os}_{arch}")
}

fn compare_versions(current: &str, latest: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    let cur = parse(current);
    let lat = parse(latest);
    for i in 0..cur.len().max(lat.len()) {
        let c = cur.get(i).copied().unwrap_or(0);
        let l = lat.get(i).copied().unwrap_or(0);
        if c < l {
            return true;
        }
        if c > l {
            return false;
        }
    }
    false
}

fn is_upgradable(
    current: Option<&str>,
    latest_version: Option<&str>,
    command_update_available: Option<bool>,
) -> bool {
    command_update_available.unwrap_or_else(|| match (current, latest_version) {
        (Some(c), Some(l)) => compare_versions(c, l),
        _ => false,
    })
}

fn redact_doctor_summary(error: &str) -> String {
    let mut summary = error
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Kimi doctor failed")
        .to_string();
    if let Some(home) = crate::util::home().to_str() {
        summary = summary.replace(home, "~");
    }
    for pattern in [
        r"(?i)(authorization|bearer|token|api[_-]?key|secret|password|credential|cookie)\s*(=|:|\s+)\S+",
        r"(?i)([?&](?:token|api[_-]?key|secret|password|credential)=)[^&\s]+",
    ] {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            let replacement = if pattern.starts_with("(?i)(authorization") {
                "$1$2<redacted>"
            } else {
                "$1<redacted>"
            };
            summary = re.replace_all(&summary, replacement).into_owned();
        }
    }
    let truncated: String = summary.chars().take(240).collect();
    if truncated.is_empty() {
        "Kimi doctor failed".to_string()
    } else {
        truncated
    }
}

fn check_health(spec: &CliSpec, installed: bool) -> Option<CliHealthStatus> {
    let command = spec.health_check_command?;
    if !installed {
        return None;
    }
    Some(match run_in_login_shell(command) {
        Ok(_) => CliHealthStatus {
            healthy: true,
            summary: None,
        },
        Err(error) => CliHealthStatus {
            healthy: false,
            summary: Some(redact_doctor_summary(&error)),
        },
    })
}

fn check_cli_version(spec: &CliSpec) -> CliVersionInfo {
    let current = if spec.name == "grok" {
        get_grok_installed_version()
    } else {
        get_installed_version(spec)
    };
    let installed = current.is_some();
    let health = check_health(spec, installed);
    // Use npm registry for npm-based CLIs, manifest URL for others (e.g. agy).
    // Kimi owns its update flow and `kimi upgrade` may prompt, so the viewer
    // intentionally has no version source or background-upgrade action for it.
    let (latest_version, error, command_update_available) = if !spec.background_upgrade {
        (None, None, None)
    } else {
        let (latest, command_update_available) = if spec.name == "grok" {
            match fetch_grok_latest_info() {
                Ok(info) => (Ok(info.latest_version), info.update_available),
                Err(e) => (Err(e), None),
            }
        } else if let Some(command) = spec.version_check_command {
            match fetch_command_latest_info(command) {
                Ok(info) => (Ok(info.latest_version), info.update_available),
                Err(e) => (Err(e), None),
            }
        } else if !spec.npm_package.is_empty() {
            (fetch_npm_latest(spec.npm_package), None)
        } else if let Some(manifest_url) = spec.manifest_url {
            (fetch_manifest_latest(manifest_url), None)
        } else {
            (Err("no version source configured".into()), None)
        };
        match latest {
            Ok(v) => (Some(v), None, command_update_available),
            Err(e) => (None, Some(e), command_update_available),
        }
    };
    let upgradable = spec.background_upgrade
        && is_upgradable(
            current.as_deref(),
            latest_version.as_deref(),
            command_update_available,
        );
    CliVersionInfo {
        cli: spec.name.to_string(),
        npm_package: spec.npm_package.to_string(),
        current_version: current,
        latest_version,
        upgradable,
        installed,
        error,
        health,
    }
}

pub fn check_all_versions() -> Vec<CliVersionInfo> {
    std::thread::scope(|s| {
        let handles: Vec<_> = CLI_SPECS
            .iter()
            .map(|spec| s.spawn(|| check_cli_version(spec)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| CliVersionInfo {
                    cli: String::new(),
                    npm_package: String::new(),
                    current_version: None,
                    latest_version: None,
                    upgradable: false,
                    installed: false,
                    error: Some("thread panic".into()),
                    health: None,
                })
            })
            .collect()
    })
}

// ---- install ----

fn resolve_install_cmd(spec: &CliSpec) -> Result<String, String> {
    // 1. Native installer (curl | bash on Unix, irm | iex on Windows)
    #[cfg(unix)]
    if let Some(cmd) = spec.install_unix {
        return Ok(cmd.to_string());
    }

    #[cfg(windows)]
    if let Some(cmd) = spec.install_windows {
        return Ok(cmd.to_string());
    }

    // 2. Homebrew (macOS / Linuxbrew)
    if let Some(brew_args) = spec.brew_upgrade {
        if run_in_login_shell("brew --version").is_ok() {
            return Ok(format!(
                "HOMEBREW_NO_INSTALL_FROM_API=1 brew install {brew_args}"
            ));
        }
    }

    // 3. npm fallback
    if !spec.npm_package.is_empty() {
        if run_in_login_shell("npm --version").is_ok() {
            return Ok(format!("npm install -g {}@latest", spec.npm_package));
        }
        return Err("npm_not_found".into());
    }

    Err("no_install_method".into())
}

pub fn install_single(cli_name: &str) -> Result<CliUpgradeResult, String> {
    let spec = find_spec(cli_name)?;
    let cmd = match resolve_install_cmd(spec) {
        Ok(cmd) => cmd,
        Err(e) => {
            return Ok(CliUpgradeResult {
                cli: spec.name.to_string(),
                success: false,
                new_version: None,
                error: Some(e),
            });
        }
    };
    match run_in_login_shell(&cmd) {
        Ok(_) => {
            let version = get_installed_version(spec);
            let success = version.is_some();
            Ok(CliUpgradeResult {
                cli: spec.name.to_string(),
                success,
                new_version: version,
                error: if success {
                    None
                } else {
                    Some("install_verification_failed".into())
                },
            })
        }
        Err(e) => Ok(CliUpgradeResult {
            cli: spec.name.to_string(),
            success: false,
            new_version: None,
            error: Some(e),
        }),
    }
}

// ---- upgrade ----

/// Detect how the CLI was installed and return the appropriate upgrade command.
///
/// Priority:
/// 1. Homebrew / Homebrew Cask → `brew upgrade <cask>`
/// 2. npm (nvm / fnm / volta / system npm) → sibling npm install -g <pkg>@latest
/// 3. Built-in update subcommand (e.g. `claude update`) as fallback
/// 4. Plain `npm install -g <pkg>@latest` as last resort
fn resolve_upgrade_cmd(spec: &CliSpec) -> String {
    let paths = find_all_paths(spec.binary);
    let first = paths.into_iter().next();
    let resolved = first.as_deref().and_then(resolve_symlink);
    let pm = resolved
        .as_deref()
        .map(detect_package_manager)
        .unwrap_or_default();

    match pm.as_str() {
        "homebrew-cask" => {
            if let Some(args) = spec.brew_upgrade {
                return format!("HOMEBREW_NO_INSTALL_FROM_API=1 brew upgrade {args}");
            }
        }
        "homebrew" => {
            let formula = spec
                .brew_upgrade
                .unwrap_or_else(|| spec.npm_package.rsplit('/').next().unwrap_or(spec.binary));
            return format!("HOMEBREW_NO_INSTALL_FROM_API=1 brew upgrade {formula}");
        }
        "nvm" | "fnm" | "volta" | "npm" => {
            if let Some(ref bin_path) = first {
                if let Some(cmd) = build_npm_upgrade(bin_path, spec.npm_package) {
                    return cmd;
                }
            }
        }
        "bun" if !spec.npm_package.is_empty() => {
            return format!("bun add -g {}@latest", spec.npm_package);
        }
        _ => {}
    }

    if let Some(builtin) = spec.builtin_update {
        return builtin.to_string();
    }

    format!("npm install -g {}@latest", spec.npm_package)
}

/// Build an npm upgrade command using the sibling npm binary from the same
/// bin directory, with NPM_CONFIG_PREFIX set so it writes to the correct tree.
fn build_npm_upgrade(bin_path: &str, npm_package: &str) -> Option<String> {
    let bin_dir = bin_path.rsplit_once('/')?.0;
    let sibling_npm = format!("{bin_dir}/npm");
    if std::path::Path::new(&sibling_npm).exists() {
        let node_root = bin_dir.rsplit_once('/').map(|(d, _)| d).unwrap_or(bin_dir);
        Some(format!(
            "NPM_CONFIG_PREFIX='{node_root}' '{sibling_npm}' install -g {npm_package}@latest"
        ))
    } else {
        None
    }
}

/// Extract a fallback upgrade command from CLI output.
/// Some CLIs (e.g. `claude update` on Homebrew installs) don't upgrade
/// directly — they print a command like "brew upgrade claude-code@latest"
/// and exit 0. We detect that and run the printed command ourselves.
fn extract_fallback_cmd(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("brew upgrade ") || trimmed.starts_with("brew reinstall ") {
            return Some(format!("HOMEBREW_NO_INSTALL_FROM_API=1 {trimmed}"));
        }
        if trimmed.starts_with("npm install ") || trimmed.starts_with("npm i ") {
            return Some(trimmed.to_string());
        }
    }
    None
}

pub fn upgrade_single(cli_name: &str) -> Result<CliUpgradeResult, String> {
    let spec = find_spec(cli_name)?;
    if !spec.background_upgrade {
        return Ok(CliUpgradeResult {
            cli: spec.name.to_string(),
            success: false,
            new_version: None,
            error: Some("background_upgrade_unsupported".into()),
        });
    }
    let prev_version = get_installed_version(spec);
    let cmd = resolve_upgrade_cmd(spec);
    match run_in_login_shell(&cmd) {
        Ok(output) => {
            if let Some(fallback) = extract_fallback_cmd(&output) {
                match run_in_login_shell(&fallback) {
                    Ok(_) => {}
                    Err(e) => {
                        return Ok(CliUpgradeResult {
                            cli: spec.name.to_string(),
                            success: false,
                            new_version: None,
                            error: Some(e),
                        });
                    }
                }
            }
            let new_version = get_installed_version(spec);
            let actually_changed = match (&prev_version, &new_version) {
                (Some(p), Some(n)) => p != n,
                _ => true,
            };
            Ok(CliUpgradeResult {
                cli: spec.name.to_string(),
                success: actually_changed,
                new_version,
                error: if actually_changed {
                    None
                } else {
                    Some("version_unchanged".into())
                },
            })
        }
        Err(e) => Ok(CliUpgradeResult {
            cli: spec.name.to_string(),
            success: false,
            new_version: None,
            error: Some(e),
        }),
    }
}

pub fn upgrade_all() -> Result<Vec<CliUpgradeResult>, String> {
    let versions = check_all_versions();
    let results: Vec<_> = versions
        .iter()
        .filter(|v| v.upgradable)
        .filter_map(|v| upgrade_single(&v.cli).ok())
        .collect();
    Ok(results)
}

// ---- diagnosis ----

#[cfg(unix)]
fn find_all_paths(binary: &str) -> Vec<String> {
    run_in_login_shell(&format!("which -a {binary} 2>/dev/null"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| l.starts_with('/'))
        .collect()
}

#[cfg(windows)]
fn find_all_paths(binary: &str) -> Vec<String> {
    // NOTE: use `where.exe`, not `where` — inside PowerShell `where` is an alias
    // for `Where-Object`, so a bare `where claude` matches nothing and the whole
    // diagnosis silently returns zero installations.
    run_in_login_shell(&format!("where.exe {binary} 2>$null"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Collapse the several launcher shims npm/native installers drop for one
/// install (`codex`, `codex.cmd`, `codex.ps1`, …, all in the same directory)
/// down to a single representative path, preferring the most directly-runnable
/// extension so the `--version` probe actually works. On Unix this is a no-op
/// pass-through; symlink de-duplication happens later via canonicalization.
#[cfg(windows)]
fn dedup_installs(paths: &[String]) -> Vec<String> {
    use std::collections::HashMap;
    let rank = |p: &str| -> u8 {
        let lower = p.to_lowercase();
        if lower.ends_with(".exe") {
            0
        } else if lower.ends_with(".cmd") {
            1
        } else if lower.ends_with(".bat") {
            2
        } else if lower.ends_with(".ps1") {
            3
        } else {
            4
        }
    };
    let mut best: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for p in paths {
        let path = std::path::Path::new(p);
        let dir = path
            .parent()
            .map(|d| d.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let key = format!("{dir}|{stem}");
        match best.get(&key) {
            Some(existing) if rank(existing) <= rank(p) => {}
            Some(_) => {
                best.insert(key, p.clone());
            }
            None => {
                order.push(key.clone());
                best.insert(key, p.clone());
            }
        }
    }
    order
        .into_iter()
        .filter_map(|k| best.get(&k).cloned())
        .collect()
}

#[cfg(unix)]
fn dedup_installs(paths: &[String]) -> Vec<String> {
    paths.to_vec()
}

#[cfg(unix)]
fn get_version_at_path(path: &str) -> Option<String> {
    let out = run_in_login_shell(&format!("'{}' --version", path.replace('\'', "'\\''"))).ok()?;
    extract_version(&out)
}

#[cfg(windows)]
fn get_version_at_path(path: &str) -> Option<String> {
    // A quoted path in PowerShell must be invoked with the call operator `&`;
    // a bare `'C:\...\x.exe' --version` is a parse error. Inside a single-quoted
    // string, a literal quote is escaped by doubling it.
    let out = run_in_login_shell(&format!("& '{}' --version", path.replace('\'', "''"))).ok()?;
    extract_version(&out)
}

fn resolve_symlink(path: &str) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

fn detect_package_manager(resolved: &str) -> String {
    let r = resolved.to_lowercase();
    if r.contains("/caskroom/") || r.contains("\\caskroom\\") {
        "homebrew-cask".into()
    } else if r.contains("/cellar/") || r.contains("\\cellar\\") {
        "homebrew".into()
    } else if r.contains("/.nvm/") || r.contains("\\.nvm\\") {
        "nvm".into()
    } else if r.contains("/.volta/") || r.contains("\\.volta\\") {
        "volta".into()
    } else if r.contains("/.fnm/") || r.contains("\\.fnm\\") {
        "fnm".into()
    } else if r.contains("/.bun/") || r.contains("\\.bun\\") {
        "bun".into()
    } else if r.contains("/node_modules/") || r.contains("\\node_modules\\") {
        "npm".into()
    } else {
        "system".into()
    }
}

/// Windows package-manager detection that does NOT depend on where node lives.
///
/// nvm-for-windows, Volta, a plain npm install, or a custom install dir all put
/// the launcher shim in a different place, so matching directory names is
/// unreliable (an earlier version hard-coded `\nvm\`, which only matched one
/// machine's layout). Instead we read the shim: npm generates `.cmd`/`.ps1`
/// launchers that invoke node against a script under `node_modules`, so the
/// presence of a `node_modules` reference inside the shim is a robust,
/// path-independent signal of an npm-global install. Real standalone binaries
/// (claude.exe, agy.exe) aren't shims and don't match → "system".
#[cfg(windows)]
fn detect_package_manager_win(raw_path: &str, resolved: Option<&str>) -> String {
    // Keep the shared string markers first (e.g. an explicit `\node_modules\`
    // in the path already answers it).
    let by_path = detect_package_manager(resolved.unwrap_or(raw_path));
    if by_path != "system" {
        return by_path;
    }
    for candidate in [resolved, Some(raw_path)].into_iter().flatten() {
        if let Ok(content) = std::fs::read_to_string(candidate) {
            if content.to_lowercase().contains("node_modules") {
                return "npm".into();
            }
        }
    }
    "system".into()
}

fn is_temp_path(path: &str) -> bool {
    (path.contains("/var/folders/") && path.contains("/T/"))
        || path.starts_with("/tmp/")
        || path.starts_with("/temp/")
}

pub fn diagnose(cli_name: &str) -> Result<CliDiagnosisResult, String> {
    let spec = find_spec(cli_name)?;
    let raw_paths = find_all_paths(spec.binary);

    // 1. Deduplicate raw paths (which -a returns duplicates when PATH has
    //    the same directory listed multiple times), then collapse the multiple
    //    launcher shims Windows installers create for one install.
    let mut seen_raw = std::collections::HashSet::new();
    let unique_paths: Vec<_> = raw_paths
        .into_iter()
        .filter(|p| seen_raw.insert(p.clone()))
        .collect();
    let unique_paths = dedup_installs(&unique_paths);

    // 2. Build installations, deduplicating by resolved (canonical) path so
    //    symlinks that point to the same binary count as one installation
    let mut seen_resolved = std::collections::HashSet::new();
    let mut installations = Vec::new();
    for path in &unique_paths {
        if is_temp_path(path) {
            continue;
        }
        let resolved = resolve_symlink(path);
        let resolved_key = resolved.clone().unwrap_or_else(|| path.clone());
        if !seen_resolved.insert(resolved_key) {
            continue;
        }
        let version = get_version_at_path(path);
        #[cfg(windows)]
        let pm = detect_package_manager_win(path, resolved.as_deref());
        #[cfg(unix)]
        let pm = resolved
            .as_deref()
            .map(detect_package_manager)
            .unwrap_or_else(|| "unknown".into());
        installations.push(CliInstallation {
            path: path.clone(),
            version,
            is_default: installations.is_empty(),
            package_manager: pm,
            resolved_path: resolved,
        });
    }

    let has_conflict = installations.len() > 1;
    Ok(CliDiagnosisResult {
        cli: spec.name.to_string(),
        binary_name: spec.binary.to_string(),
        installations,
        has_conflict,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_version() {
        assert_eq!(
            extract_version("2.1.187 (Claude Code)"),
            Some("2.1.187".into())
        );
        assert_eq!(extract_version("codex-cli 0.142.3"), Some("0.142.3".into()));
        assert_eq!(extract_version("0.43.0"), Some("0.43.0".into()));
        assert_eq!(extract_version("no version here"), None);
    }

    #[test]
    fn kimi_cli_has_official_installer_and_never_background_upgrades() {
        let kimi = find_spec("kimi").unwrap();
        assert_eq!(kimi.binary, "kimi");
        assert_eq!(
            kimi.install_unix,
            Some("curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash")
        );
        assert_eq!(
            kimi.install_windows,
            Some("irm https://code.kimi.com/kimi-code/install.ps1 | iex")
        );
        assert!(!kimi.background_upgrade);
        assert_eq!(kimi.health_check_command, Some("kimi doctor"));
    }

    #[test]
    fn doctor_failure_summary_redacts_secrets_and_home_path() {
        let home = crate::util::home();
        let input = format!(
            "config {} token=super-secret-value\nignored",
            home.join(".kimi-code/config.toml").display()
        );
        let summary = redact_doctor_summary(&input);
        assert!(summary.contains("~/.kimi-code/config.toml"));
        assert!(summary.contains("token=<redacted>"));
        assert!(!summary.contains("super-secret-value"));
    }

    #[test]
    fn grok_version_check_reads_latest_version_without_exposing_other_fields() {
        let check = extract_command_latest_info(
            r#"{"currentVersion":"1.0.4","latestVersion":"1.0.5","updateAvailable":true}"#,
        )
        .unwrap();
        assert_eq!(check.latest_version, "1.0.5");
        assert_eq!(check.update_available, Some(true));
        assert!(extract_command_latest_info("{\"error\":null}").is_err());
    }

    #[test]
    fn test_compare_versions() {
        assert!(compare_versions("2.1.187", "2.1.197"));
        assert!(!compare_versions("2.1.197", "2.1.187"));
        assert!(!compare_versions("2.1.187", "2.1.187"));
        assert!(compare_versions("0.43.0", "0.49.0"));
        assert!(compare_versions("0.142.3", "0.142.5"));
    }

    #[test]
    fn command_update_available_overrides_version_comparison() {
        assert!(is_upgradable(Some("1.0.4"), Some("1.0.5"), Some(true)));
        assert!(!is_upgradable(Some("1.0.4"), Some("1.0.5"), Some(false)));
        assert!(is_upgradable(Some("1.0.4"), Some("1.0.5"), None));
    }

    #[test]
    fn test_detect_package_manager() {
        assert_eq!(
            detect_package_manager("/opt/homebrew/Caskroom/claude-code/2.1.187/claude"),
            "homebrew-cask"
        );
        assert_eq!(
            detect_package_manager(
                "/Users/u/.nvm/versions/node/v22.21.1/lib/node_modules/@openai/codex/bin/codex.js"
            ),
            "nvm"
        );
        assert_eq!(detect_package_manager("/Users/u/.volta/bin/codex"), "volta");
        assert_eq!(
            detect_package_manager("/usr/local/lib/node_modules/@openai/codex/bin/codex"),
            "npm"
        );
        assert_eq!(detect_package_manager("/usr/bin/claude"), "system");
        // A node_modules segment marks an npm install on any OS / any dir.
        assert_eq!(
            detect_package_manager("X:\\anything\\node_modules\\pkg\\bin\\tool.cmd"),
            "npm"
        );
        // Windows package-manager detection no longer guesses from the install
        // directory (that only ever matched one machine's layout) — it reads the
        // shim in detect_package_manager_win, covered by manual/live testing
        // since it needs real files on disk.
    }

    #[cfg(windows)]
    #[test]
    fn test_dedup_installs_collapses_shims() {
        // The values below are synthetic placeholders, not any real install dir.
        // npm drops `foo` + `foo.cmd` in the SAME dir for one install; they must
        // collapse to a single entry, preferring the runnable `.cmd`.
        let dir = "X:\\bin";
        let paths = vec![format!("{dir}\\foo"), format!("{dir}\\foo.cmd")];
        assert_eq!(dedup_installs(&paths), vec![format!("{dir}\\foo.cmd")]);

        // The SAME binary name in two DIFFERENT dirs is two real installs.
        let paths = vec![
            "X:\\bin-a\\foo.cmd".to_string(),
            "Y:\\bin-b\\foo.cmd".to_string(),
        ];
        assert_eq!(dedup_installs(&paths).len(), 2);
    }

    #[test]
    fn test_extract_fallback_cmd() {
        let output = "Current version: 2.1.187\n\
                       Checking for updates to latest version...\n\
                       \n\
                       Claude is managed by Homebrew.\n\
                       Update available: 2.1.187 → 2.1.197\n\
                       \n\
                       To update, run:\n\
                         brew upgrade claude-code@latest\n";
        assert_eq!(
            extract_fallback_cmd(output),
            Some("HOMEBREW_NO_INSTALL_FROM_API=1 brew upgrade claude-code@latest".into())
        );

        assert_eq!(extract_fallback_cmd("Updated successfully!"), None);
        assert_eq!(
            extract_fallback_cmd("  npm install -g @openai/codex@latest"),
            Some("npm install -g @openai/codex@latest".into())
        );
    }
}
