// 嵌入终端 (in-app TUI) —— 在窗口里直接跑 CLI，免去切到 Terminal.app。
//
// 设计：
//   - 用 `portable-pty` 开一个伪终端 —— Unix 走 forkpty + openpty，Windows 走 ConPTY
//     (Win10 1809+，比这老的版本会在 openpty 时报错，前端能看到原话）。
//   - 把用户登录 shell 拉起来，让它 `cd '<cwd>'` 后跑 resume CLI。
//     · POSIX (`macOS / Linux`)：`$SHELL -l -i -c "cd '<cwd>' && <cli>"`。
//       `-l + -i` 同时给：login shell source `.zprofile` / `.bash_profile`，interactive
//       shell source `.zshrc` / `.bashrc`。npm global / nvm / fnm / volta 通常把 PATH
//       export 在 rc 文件里，少一个都可能找不到 claude / codex。
//     · Windows：`<pwsh|powershell>.exe -NoLogo -Command "Set-Location -LiteralPath '<cwd>'; <cli>"`。
//       优先 `pwsh.exe`（PS7，用户默认想用的）——装了才用，没装回退 Win10+ 自带的
//       `powershell.exe`（5.1）以兼容空机器；见 agent_command::windows_powershell_exe。
//       `-Command` 默认会 load `$PROFILE`，nvm-windows / volta-win 在 profile 里改的 PATH
//       也能拿到。msi / choco 安装的 CLI 直接走系统 PATH，无 profile 也能找到。
//   - 给每个活跃 PTY 分配一个 `u64` id，前端拿着 id 调 write / resize / kill。
//   - 派一个 reader 线程不停读 master 端字节，base64 后 emit 给前端 xterm 绘制。
//     另外派一个 waiter 线程 try_wait 子进程退出，emit 一次 `pty://exit`。
//   - 同进程允许多个 PTY 并存（用户可能边切会话边在另一条上跑 CLI）—— 状态藏在
//     `OnceLock<Mutex<HashMap<id, Arc<PtyHandle>>>>` 里。
//
// 前端事件契约：
//   pty://data   { id: u64, base64: string }     PTY stdout 的原始字节
//   pty://exit   { id: u64, code: i32 }          子进程退出，前端关闭 pane
//
// 不在这里读 JSONL —— 真正的会话内容由 watch.rs 的 file-tail 通道继续推回 ChatView，
// 用户从 TUI 切回 view 模式时直接 read_session 整段重拉即可。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::agent_command::AgentCommand;

#[derive(Clone, Copy)]
enum PtyColorScheme {
    Light,
    Dark,
}

impl PtyColorScheme {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("dark") => Self::Dark,
            _ => Self::Light,
        }
    }

    fn colorfgbg(self) -> &'static str {
        match self {
            Self::Light => "0;15",
            Self::Dark => "15;0",
        }
    }
}

struct PtyHandle {
    /// 保留 master 主要为了 resize（reader / writer 都已克隆出去）。
    master: Mutex<Box<dyn MasterPty + Send>>,
    /// 写端：前端键盘输入解 base64 后写进来；Mutex 保护并发输入（罕见但稳）。
    writer: Mutex<Box<dyn Write + Send>>,
    /// 子进程句柄：kill 时用；waiter 线程 try_wait 走一份独立的弱引用避免长锁。
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    /// resume 会话的独占写入租约；纯 shell / new session 没有。
    _session_lease: Option<crate::runtime::SessionLease>,
}

/// Agent PTY 启动所需的可选参数；收拢后避免调用方传递一串易混淆的基础类型。
pub struct PtySpawnOptions<'a> {
    pub cols: u16,
    pub rows: u16,
    pub color_scheme: Option<&'a str>,
    pub use_reclaude: bool,
    pub session: Option<(String, String)>,
}

static PTYS: OnceLock<Mutex<HashMap<u64, Arc<PtyHandle>>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn map() -> &'static Mutex<HashMap<u64, Arc<PtyHandle>>> {
    PTYS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Serialize, Clone)]
struct DataPayload {
    id: u64,
    /// base64(原始 PTY 字节)。让前端拿 raw 字节喂给 xterm，避免 utf-8 截断 / 控制字符变形。
    base64: String,
}

#[derive(Serialize, Clone)]
struct ExitPayload {
    id: u64,
    code: i32,
}

/// 按 OS 组装 shell 调用。
///
/// POSIX (macOS / Linux) → `$SHELL -l -i -c "cd '<cwd>' && <cli>"`，让 login + interactive
/// shell 同时 source 两套 rc 文件，把 nvm / fnm / volta / npm-global 在 rc 里 export 的
/// PATH 都拉进来。SHELL 缺省时 macOS 回退 zsh、Linux 回退 bash —— 各自系统默认。
///
/// Windows → `<pwsh|powershell>.exe -NoLogo -Command "Set-Location -LiteralPath '<cwd>'; <cli>"`。
/// 优先 PS7（`pwsh.exe`），没装才回退内置的 `powershell.exe`（PS 5.1，Win10+ 自带）。
/// `-Command` 默认会 load `$PROFILE` —— nvm-windows / volta 在 profile 里改的 PATH 能
/// 拿到；msi / choco 装的 CLI 直接走系统 PATH。
///
/// `cwd` 已在 lib.rs 校验过是真实目录；`command` 已由 agent 构造并负责平台渲染。
#[cfg(unix)]
fn build_shell_command(
    cwd: &str,
    command: &AgentCommand,
    color_scheme: PtyColorScheme,
    use_reclaude: bool,
) -> CommandBuilder {
    #[cfg(target_os = "macos")]
    const DEFAULT_SHELL: &str = "/bin/zsh";
    #[cfg(not(target_os = "macos"))]
    const DEFAULT_SHELL: &str = "/bin/bash";

    let shell = std::env::var("SHELL").unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let command = if command.program() == "grok" {
        let grok = crate::util::home().join(".grok/bin/grok");
        command
            .clone()
            .with_program(grok.to_string_lossy().into_owned())
    } else {
        command.clone()
    };
    let cli = if use_reclaude {
        format!("'reclaude' {}", command.to_posix_shell())
    } else {
        command.to_posix_shell()
    };
    let inner = format!("cd {} && {}", crate::agent_command::posix_quote(cwd), cli);

    let mut cmd = CommandBuilder::new(&shell);
    cmd.arg("-l");
    cmd.arg("-i");
    cmd.arg("-c");
    cmd.arg(&inner);
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("npm_config_prefix");
    let home = crate::util::home();
    let grok_bin = home.join(".grok/bin");
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(grok_bin).chain(std::env::split_paths(&inherited_path)),
    )
    .map_err(|_| ())
    .unwrap_or(inherited_path);
    cmd.env("PATH", path);
    cmd.env("HOME", &home);
    cmd.env("GROK_HOME", home.join(".grok"));
    cmd.env("COLORTERM", "truecolor");
    cmd.env("COLORFGBG", color_scheme.colorfgbg());
    cmd.cwd(cwd);
    cmd
}

#[cfg(windows)]
fn build_shell_command(
    cwd: &str,
    command: &AgentCommand,
    color_scheme: PtyColorScheme,
    use_reclaude: bool,
) -> CommandBuilder {
    // PowerShell 单引号字面串里 ' 用 '' 转义。装了 PS7 就用 pwsh，否则回退自带的 5.1。
    let mut cmd = CommandBuilder::new(crate::agent_command::windows_powershell_exe());
    cmd.arg("-NoLogo");
    // -ExecutionPolicy Bypass（仅本进程）：resume 会 `& 'codex' ...` 跑 npm 装的 .ps1 垫片，
    // Win 默认策略 Restricted 会以 UnauthorizedAccess 拒绝，Bypass 放行且不改系统策略。
    cmd.arg("-ExecutionPolicy");
    cmd.arg("Bypass");
    cmd.arg("-Command");
    cmd.arg(crate::agent_command::powershell_set_location_and_run(
        cwd,
        command,
        use_reclaude,
    ));
    // ConPTY 自己处理 VT 序列；TERM 对 Win 上的 Node CLI（claude / codex）也无害。
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("COLORFGBG", color_scheme.colorfgbg());
    // 与 v0.1.12 一致：不在这里注入 PATH。powershell_set_location_and_run 已在 shell 内先跑
    // powershell_refresh_path()（$processPath 打头继承），再 `& <cli>`，node/claude 都能找到。
    cmd.cwd(cwd);
    cmd
}

/// 纯交互式 shell（不带 -c），用于"新建终端"场景。
#[cfg(unix)]
fn build_interactive_shell(cwd: &str, color_scheme: PtyColorScheme) -> CommandBuilder {
    #[cfg(target_os = "macos")]
    const DEFAULT_SHELL: &str = "/bin/zsh";
    #[cfg(not(target_os = "macos"))]
    const DEFAULT_SHELL: &str = "/bin/bash";

    let shell = std::env::var("SHELL").unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.arg("-l");
    cmd.arg("-i");
    // Login/interactive startup files are allowed to rewrite PATH. Re-enter a
    // clean interactive shell after they finish so the managed Grok binary is
    // still the first command resolution candidate. This does not freeze the
    // rest of PATH, so pnpm/node changes from the user's shell remain visible.
    cmd.arg("-c");
    let home = crate::util::home();
    let grok_bin = home.join(".grok/bin");
    let grok = grok_bin.join("grok");
    cmd.arg(format!(
        "export PATH={}:$PATH; export HOME={}; export GROK_HOME={}; unset GROK_CLI_BASE_URL GROK_CHANNEL GROK_VERSION GROK_MINIMUM_VERSION GROK_MAXIMUM_VERSION GROK_REQUIRED_MINIMUM_VERSION GROK_REQUIRED_MAXIMUM_VERSION XAI_API_BASE_URL; function grok {{ env -u GROK_CLI_BASE_URL -u GROK_CHANNEL -u GROK_VERSION -u GROK_MINIMUM_VERSION -u GROK_MAXIMUM_VERSION -u GROK_REQUIRED_MINIMUM_VERSION -u GROK_REQUIRED_MAXIMUM_VERSION -u XAI_API_BASE_URL {} \"$@\"; }}; rehash 2>/dev/null; exec {} -i",
        crate::agent_command::posix_quote(&grok_bin.to_string_lossy()),
        crate::agent_command::posix_quote(&home.to_string_lossy()),
        crate::agent_command::posix_quote(&home.join(".grok").to_string_lossy()),
        crate::agent_command::posix_quote(&grok.to_string_lossy()),
        crate::agent_command::posix_quote(&shell),
    ));
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("npm_config_prefix");

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(grok_bin).chain(std::env::split_paths(&inherited_path)),
    )
    .unwrap_or(inherited_path);
    cmd.env("PATH", path);
    cmd.env("HOME", &home);
    cmd.env("GROK_HOME", home.join(".grok"));
    cmd.env("COLORTERM", "truecolor");
    cmd.env("COLORFGBG", color_scheme.colorfgbg());
    cmd.cwd(cwd);
    cmd
}

#[cfg(windows)]
fn build_interactive_shell(cwd: &str, color_scheme: PtyColorScheme) -> CommandBuilder {
    // 装了 PS7 就用 pwsh，否则回退自带的 5.1。
    let mut cmd = CommandBuilder::new(crate::agent_command::windows_powershell_exe());
    cmd.arg("-NoLogo");
    // 交互式里用户随手跑 claude/codex 也会碰到 .ps1 垫片，同样放行执行策略（仅本进程）。
    cmd.arg("-ExecutionPolicy");
    cmd.arg("Bypass");
    // 曾经这里裸起 powershell、纯靠继承 GUI 进程的环境块 PATH。EXE/NSIS 装的场景 explorer
    // 传下来的 PATH 完整，实测好使；但 MSI (WiX) 的 advertised-shortcut 走 Windows Installer
    // 上下文启动，继承到的环境块残缺，nvm 装的 node/npm 段丢失 → 新建终端里 `npm` 报"不是命令"。
    // 所以和 resume / 版本检测走同一套：先跑 powershell_refresh_path() 从注册表补齐并展开
    // %NVM_SYMLINK% 等占位符，再靠 -NoExit 落回交互提示符。$processPath 打头，继承到的完整
    // PATH 仍优先，刷新只增不减，不会覆盖本来就好用的继承 PATH。
    cmd.arg("-NoExit");
    cmd.arg("-Command");
    // shell_init = 静默写一份 PATH 诊断快照到 %TEMP%\sv-pathdiag.txt + 刷新 PATH。
    cmd.arg(crate::agent_command::powershell_shell_init());
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("COLORFGBG", color_scheme.colorfgbg());
    cmd.cwd(cwd);
    cmd
}

/// 拉起 PTY 跑 OS 对应的 shell 调用（详见 [`build_shell_command`]）。
///
/// `command` 已由 agent 构造；`cwd` 已在 lib.rs 里校验是真实目录。返回新 PTY 的内部 id。
pub fn spawn(
    app: AppHandle,
    cwd: String,
    command: AgentCommand,
    options: PtySpawnOptions<'_>,
) -> Result<u64, String> {
    let cmd = build_shell_command(
        &cwd,
        &command,
        PtyColorScheme::parse(options.color_scheme),
        options.use_reclaude,
    );
    spawn_raw(app, &cwd, cmd, options.cols, options.rows, options.session)
}

/// 启动一个纯 shell PTY（不跑任何 agent CLI），用于在项目目录里执行任意命令。
pub fn spawn_shell(
    app: AppHandle,
    cwd: String,
    cols: u16,
    rows: u16,
    color_scheme: Option<&str>,
) -> Result<u64, String> {
    let cmd = build_interactive_shell(&cwd, PtyColorScheme::parse(color_scheme));
    spawn_raw(app, &cwd, cmd, cols, rows, None)
}

fn spawn_raw(
    app: AppHandle,
    cwd: &str,
    cmd: CommandBuilder,
    cols: u16,
    rows: u16,
    session: Option<(String, String)>,
) -> Result<u64, String> {
    if !std::path::Path::new(cwd).is_dir() {
        return Err("项目目录已不存在，无法启动终端".into());
    }
    let spawn_permit = crate::runtime::spawn_permit()?;
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let session_lease = session
        .as_ref()
        .map(|(agent, session_id)| {
            crate::runtime::acquire_session(
                agent,
                session_id,
                crate::runtime::SessionOwner::Tui(id),
            )
        })
        .transpose()?;
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.max(8),
            cols: cols.max(20),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn failed: {e}"))?;
    // slave 端在父进程留着的话，PTY 永远不会 EOF；spawn 完立刻 drop。
    drop(pair.slave);

    let pid = match child.process_id() {
        Some(pid) => pid,
        None => {
            stop_child(child.as_mut());
            return Err("spawned PTY child has no process id".to_string());
        }
    };
    if let Err(error) = crate::process_tree::register(pid) {
        stop_child(child.as_mut());
        return Err(error);
    }
    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            stop_child(child.as_mut());
            return Err(format!("clone reader failed: {error}"));
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            stop_child(child.as_mut());
            return Err(format!("take writer failed: {error}"));
        }
    };

    let handle = Arc::new(PtyHandle {
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
        _session_lease: session_lease,
    });
    match map().lock() {
        Ok(mut ptys) => {
            ptys.insert(id, handle.clone());
        }
        Err(error) => {
            stop_handle(handle.as_ref());
            return Err(error.to_string());
        }
    }
    drop(spawn_permit);

    // ---- reader 线程：阻塞 read → base64 → emit ----
    let app_for_reader = app.clone();
    thread::spawn(move || reader_loop(app_for_reader, id, reader));

    // ---- waiter 线程：try_wait 退出码后 emit + 清理 ----
    let app_for_wait = app.clone();
    thread::spawn(move || waiter_loop(app_for_wait, id));

    Ok(id)
}

fn reader_loop(app: AppHandle, id: u64, mut reader: Box<dyn Read + Send>) {
    // PTY 输出合并：TUI（如 Claude Code 的 ink 界面）整屏重绘会被内核拆成一串小块。
    // 逐块 emit 的话每块都吃一次 IPC 延迟，重绘被摊到多个渲染帧上 —— 前端能看见
    // 「光标飞到状态栏画字再飞回来」的中间态（光标抖动）。这里把阻塞读挪进子线程，
    // 本线程在 2ms 静默窗口内合并后续块（总时长 16ms / 128KB 封顶）再一次性 emit，
    // 一次重绘基本落在同一个事件里，前端一帧画完只见最终态。
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF —— slave 端被关 / 子进程退出。
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                // 任何 IO 错误（连接关闭 / 中断）一律退出。Waiter 线程会负责 emit exit。
                Err(_) => break,
            }
        }
    });

    while let Ok(first) = rx.recv() {
        let mut acc = first;
        let deadline = Instant::now() + Duration::from_millis(16);
        while acc.len() < 128 * 1024 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let quiet = Duration::from_millis(2).min(deadline - now);
            match rx.recv_timeout(quiet) {
                Ok(chunk) => acc.extend_from_slice(&chunk),
                Err(_) => break, // 静默 2ms 或读线程退出 —— 都先把手上的发出去
            }
        }
        let payload = DataPayload {
            id,
            base64: B64.encode(&acc),
        };
        if app.emit("pty://data", payload).is_err() {
            // 事件 emit 失败通常意味着 app 在 teardown —— 直接 break。
            break;
        }
    }
}

fn waiter_loop(app: AppHandle, id: u64) {
    loop {
        // 锁尽量短：拿一次 try_wait，没结果就 sleep。
        let res = {
            let arc = match map().lock().ok().and_then(|m| m.get(&id).cloned()) {
                Some(a) => a,
                None => return, // kill_pty 已经把它移走了，啥也别干。
            };
            let mut child = match arc.child.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            child.try_wait()
        };
        match res {
            Ok(Some(status)) => {
                let code = status.exit_code() as i32;
                let _ = app.emit("pty://exit", ExitPayload { id, code });
                if let Ok(mut m) = map().lock() {
                    m.remove(&id);
                }
                return;
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(150));
            }
            Err(_) => {
                let _ = app.emit("pty://exit", ExitPayload { id, code: -1 });
                if let Ok(mut m) = map().lock() {
                    m.remove(&id);
                }
                return;
            }
        }
    }
}

pub fn write(id: u64, base64_data: &str) -> Result<(), String> {
    let arc = {
        let m = map().lock().map_err(|e| e.to_string())?;
        m.get(&id)
            .cloned()
            .ok_or_else(|| "PTY not found".to_string())?
    };
    let bytes = B64
        .decode(base64_data)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    let mut w = arc.writer.lock().map_err(|e| e.to_string())?;
    w.write_all(&bytes).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn resize(id: u64, cols: u16, rows: u16) -> Result<(), String> {
    let arc = {
        let m = map().lock().map_err(|e| e.to_string())?;
        m.get(&id)
            .cloned()
            .ok_or_else(|| "PTY not found".to_string())?
    };
    let master = arc.master.lock().map_err(|e| e.to_string())?;
    master
        .resize(PtySize {
            rows: rows.max(8),
            cols: cols.max(20),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("resize failed: {e}"))
}

fn stop_child(child: &mut (dyn portable_pty::Child + Send + Sync)) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    if let Some(pid) = child.process_id() {
        crate::process_tree::terminate(pid);
    }
    let _ = child.kill();
    // wait 一下确保进程真死了，避免僵尸；失败不向上抛，关闭本身保持幂等。
    let _ = child.wait();
}

fn stop_handle(handle: &PtyHandle) {
    if let Ok(mut child) = handle.child.lock() {
        stop_child(child.as_mut());
    }
}

pub fn kill(id: u64) -> Result<(), String> {
    // 先把 entry 拿走 —— waiter 线程下一轮 try_wait 时会发现 entry 不见了直接 return，
    // 避免它在 child 已经被 drop 之后还去 emit 一个奇怪的 exit。
    let arc = {
        let mut m = map().lock().map_err(|e| e.to_string())?;
        m.remove(&id)
    };
    let Some(arc) = arc else {
        return Ok(());
    };
    stop_handle(arc.as_ref());
    Ok(())
}

/// 应用退出 / 在线更新前停止全部内嵌终端及其 CLI 后代。
pub fn kill_all() {
    let handles: Vec<Arc<PtyHandle>> = {
        let mut ptys = map().lock().unwrap_or_else(|e| e.into_inner());
        ptys.drain().map(|(_, handle)| handle).collect()
    };
    for handle in handles {
        stop_handle(handle.as_ref());
    }
}
