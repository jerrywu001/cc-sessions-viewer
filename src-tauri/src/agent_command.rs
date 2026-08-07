#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommand {
    program: String,
    args: Vec<String>,
    extra_args: String,
}

impl AgentCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            extra_args: String::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn with_extra_args(mut self, extra: &str) -> Self {
        self.extra_args = extra.trim().to_string();
        self
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    pub fn to_posix_shell(&self) -> String {
        let mut parts =
            Vec::with_capacity(1 + self.args.len() + usize::from(!self.extra_args.is_empty()));
        parts.push(posix_quote(&self.program));
        parts.extend(self.args.iter().map(|arg| posix_quote(arg)));
        if !self.extra_args.is_empty() {
            parts.push(self.extra_args.clone());
        }
        parts.join(" ")
    }

    /// `wrapper` 给出时用它做进程包装器（`& 'reclaude' 'claude' ...`）：`&` 调用算子跑
    /// wrapper，原 program/args 全成 wrapper 的参数。与 posix 侧 `'reclaude' <cli>` 同款语义。
    #[cfg(target_os = "windows")]
    pub fn to_powershell(&self, wrapper: Option<&str>) -> String {
        let mut parts = Vec::with_capacity(
            2 + usize::from(wrapper.is_some())
                + self.args.len()
                + usize::from(!self.extra_args.is_empty()),
        );
        parts.push("&".to_string());
        if let Some(wrapper) = wrapper {
            parts.push(powershell_quote(wrapper));
        }
        parts.push(powershell_quote(&self.program));
        parts.extend(self.args.iter().map(|arg| powershell_quote(arg)));
        if !self.extra_args.is_empty() {
            parts.push(self.extra_args.clone());
        }
        parts.join(" ")
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "windows")]
pub fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `use_reclaude`：GUI 聊天 / 内嵌终端里把命令包一层 `reclaude`，走 reclaude 守护进程的
/// 鉴权 + 代理链路（与 posix 侧一致）。外部终端 resume 传 `false`。
#[cfg(target_os = "windows")]
pub fn powershell_set_location_and_run(
    cwd: &str,
    command: &AgentCommand,
    use_reclaude: bool,
) -> String {
    let cwd = powershell_quote(cwd);
    let wrapper = if use_reclaude { Some("reclaude") } else { None };
    format!(
        "{}; Set-Location -LiteralPath {cwd}; {}",
        powershell_refresh_path(),
        command.to_powershell(wrapper)
    )
}

/// 在 PowerShell 会话内把 PATH 重新拼一遍并**解析符号链接目录**，让内嵌终端 / resume /
/// 版本检测都能稳定找到 node / npm / claude / codex。
///
/// 拼装来源（去重后逐段并入）：进程现有 PATH（已由 OS 展开，最可靠）打头，再补注册表
/// User + Machine PATH，最后兜一个 NVM_SYMLINK。注册表两段套 ExpandEnvironmentVariables
/// 先把 REG_EXPAND_SZ 的 `%VAR%` 展开；`+ ''` 兜住可能为 $null 的值免得抛异常。
///
/// **关键**：MSI（WiX advertised-shortcut）拉起的进程**无法穿过目录符号链接**去查找命令
/// —— nvm-for-windows 的 `node` 目录（NVM_SYMLINK，如 `D:\nvm\nodejs`）就是个指向真实版本
/// 目录（`...\vX.Y.Z`）的符号链接，于是即便它在 PATH 里，`node`/`npm`/`codex` 仍全部
/// CommandNotFound；而 EXE/NSIS 拉起的进程能穿透，原生 node（真实目录、无 reparse）也不
/// 受影响 —— 这解释了「MSI + nvm 挂、EXE 或原生都正常」。修法通用、不写死任何路径：遍历
/// 拼好的每个目录，凡是 ReparsePoint 就把它 `.Target` 解析出的**真实目录**一并加进 PATH，
/// 命令查找走真实目录（无 reparse）即可命中。`.Target` 只读 reparse 数据、不需要穿透，
/// 所以在穿不过链接的上下文里依旧读得到。对非符号链接目录（原生安装）该分支不触发，零影响。
#[cfg(target_os = "windows")]
pub fn powershell_refresh_path() -> &'static str {
    "$machinePath = [Environment]::ExpandEnvironmentVariables(([Environment]::GetEnvironmentVariable('Path', 'Machine') + '')); \
     $userPath = [Environment]::ExpandEnvironmentVariables(([Environment]::GetEnvironmentVariable('Path', 'User') + '')); \
     $processPath = [Environment]::GetEnvironmentVariable('Path', 'Process'); \
     $nvmSym = @([Environment]::GetEnvironmentVariable('NVM_SYMLINK','User'), [Environment]::GetEnvironmentVariable('NVM_SYMLINK','Machine'), $env:NVM_SYMLINK) | Where-Object { $_ } | Select-Object -First 1; \
     $dirs = @($processPath, $userPath, $machinePath, $nvmSym) | Where-Object { $_ } | ForEach-Object { $_ -split ';' } | Where-Object { $_ }; \
     $out = New-Object System.Collections.Generic.List[string]; \
     foreach ($d in $dirs) { $out.Add($d); try { $it = Get-Item -LiteralPath $d -Force -ErrorAction Stop; if (($it.Attributes -band [IO.FileAttributes]::ReparsePoint) -and $it.Target) { $t = @($it.Target)[0]; if (-not [IO.Path]::IsPathRooted($t)) { $t = [IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $d) $t)) }; $out.Add($t) } } catch { } }; \
     $env:Path = ($out | Where-Object { $_ } | Select-Object -Unique) -join ';'"
}

/// 内嵌「新建终端」启动时先跑的初始化：先 [`powershell_refresh_path`] 刷新+解析符号链接，
/// 再把一份含**刷新后最终 PATH**的诊断快照静默写到 %TEMP%\sv-pathdiag.txt（排查 MSI 启动
/// 上下文下命令找不到的问题）。诊断写入用 try/catch 包住，失败也不影响交互提示符。
#[cfg(target_os = "windows")]
pub fn powershell_shell_init() -> String {
    let diag = "; $d = Join-Path $env:TEMP 'sv-pathdiag.txt'; \
        try { (\"PSVER=\" + $PSVersionTable.PSVersion + \"`nEXE=\" + \
        [System.Diagnostics.Process]::GetCurrentProcess().MainModule.FileName + \
        \"`nnode=\" + (Get-Command node -ErrorAction SilentlyContinue).Source + \
        \"`nnpm=\" + (Get-Command npm -ErrorAction SilentlyContinue).Source + \
        \"`nFINAL_PATH=\" + $env:Path) | \
        Out-File -Encoding utf8 $d } catch { }";
    format!("{}{diag}", powershell_refresh_path())
}

/// 选用哪个 PowerShell 可执行文件：优先 PowerShell 7（`pwsh.exe`，用户默认想用的那个），
/// 装了才用；没装则回退到系统自带的 Windows PowerShell 5.1（`powershell.exe`）以兼容空机器。
/// 检测只扫继承到的 PATH（PS7 安装器会把自己写进 Machine PATH），不另起子进程。
#[cfg(target_os = "windows")]
pub fn windows_powershell_exe() -> &'static str {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join("pwsh.exe").is_file() {
                return "pwsh.exe";
            }
        }
    }
    "powershell.exe"
}

/// 将 PowerShell 命令编码为 `-EncodedCommand` 所需的 Base64 (UTF-16LE)。
/// 打包后的 .app 经 `cmd /c start "" powershell.exe -Command "..."` 启动时，
/// CMD 的引号层会吞掉 `$`、`@`、`&` 等特殊字符，导致 PATH 刷新失败。
/// `-EncodedCommand` 完全绕过引号解析，是 Windows 上最可靠的传参方式。
#[cfg(target_os = "windows")]
pub fn powershell_encoded_command(ps_cmd: &str) -> String {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let utf16le: Vec<u8> = ps_cmd
        .encode_utf16()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    B64.encode(utf16le)
}
