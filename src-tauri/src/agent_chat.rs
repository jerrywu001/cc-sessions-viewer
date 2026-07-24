// 程序化聊天（GUI chat）—— 用纯管道子进程跑 agent 的 headless stream-json 模式，
// 逐行读 JSON 事件直接喂给前端复用的 `Block`/ChatView 渲染。
//
// 与 `pty.rs` 的关系：
//   - pty.rs 服务「窗口内 TUI resume / shell」—— 走伪终端，处理 ANSI / 光标，给 xterm。
//   - agent_chat.rs 服务「干净聊天框」—— 走 `Stdio::piped()` 纯管道，没有 TUI 控制字符，
//     stdout 每一行是一个 JSON 事件，由各 agent 的 `parse_chat_line` 归一成 `ChatEvent`。
//   两者并存，互不影响；结构刻意对齐（HashMap<id, Arc<Handle>> + reader/waiter 线程）。
//
// 设计：
//   - 通过用户登录 shell 拉起 CLI（`$SHELL -l -i -c "cd '<cwd>' && <cli>"` / powershell），
//     与 pty.rs 同款，确保 nvm / fnm / volta / npm-global 的 PATH 都能拿到 claude。
//   - stdin 持续喂 `{"type":"user","message":{...}}`（含可选 image 块）；进程长驻直到 stdin
//     关闭 / 被 kill。
//   - reader 线程逐行读 stdout → `source.parse_chat_line(line)` → emit 对应事件。
//   - stderr 线程收诊断行（emit `agent-chat://stderr`，便于排障）。
//   - waiter 线程 try_wait 退出码后 emit 一次 `agent-chat://exit` 并清理。
//
// 前端事件契约：
//   agent-chat://event   { chatId, msg }              一条解析好的 Msg（assistant / tool_result）
//   agent-chat://init    { chatId, sessionId }        子进程报告的 session id（定位 JSONL / 续聊）
//   agent-chat://result  { chatId, ok, usage }        一轮回答结束（驱动 turn 门控）
//   agent-chat://stderr  { chatId, line }             子进程 stderr 诊断行
//   agent-chat://exit    { chatId, code }             子进程退出
//
// webview 刷新时后端进程不杀 —— 前端重连（list_running_chats → reconnect）。

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::agent_command::AgentCommand;
use crate::agents::{self, ChatEvent, ChatProcessModel};
use crate::types::{ChatImageInput, UsageSummary};

/// 一个 chat「会话」的句柄。两种进程模型对应两个变体（见 [`ChatProcessModel`]）。
#[allow(clippy::large_enum_variant)]
enum ChatHandle {
    /// 长驻进程（Claude）：start 时 spawn，send 写 stdin，waiter 监控退出。
    LongLived {
        /// 该会话的 agent —— send 时据此取 `chat_encode_input`。
        agent: String,
        /// 写端：用户消息 JSON 逐行写进来；Mutex 保护并发输入。
        stdin: Mutex<ChildStdin>,
        /// 子进程句柄：stop 时 kill；waiter 线程走短锁 try_wait 避免长占。
        child: Mutex<Child>,
    },
    /// 一轮一进程（Codex）：start 不 spawn，send 时 spawn 一个 resume 进程。
    OneShot {
        /// 用于 send 时 spawn turn 进程 + emit 事件。
        app: AppHandle,
        agent: String,
        cwd: String,
        /// 上一轮回填的 session/thread id —— 下一轮 resume 用。
        session_id: Mutex<Option<String>>,
        /// 当前在跑的那一轮子进程（stop 时 kill）。
        current: Mutex<Option<Child>>,
        /// Codex exec 没有 Claude 那种 stdin 控制协议。权限不足时先暂停本轮，把原 prompt
        /// 暂存在这里；前端批准后用更高权限重跑同一轮。
        pending_approval: Mutex<Option<OneShotApproval>>,
        /// “始终允许”只在当前 GUI chat 会话内生效：同一命令前缀再次被 sandbox 拦住时自动重跑。
        approved_command_prefixes: Mutex<Vec<String>>,
        use_reclaude: bool,
    },
    /// Codex rich-client protocol (`codex app-server --stdio`). This is the path that can
    /// surface real approval requests instead of headless `codex exec` failures.
    CodexAppServer { shared: Arc<CodexAppServerShared> },
}

#[derive(Clone)]
struct OneShotTurnSpec {
    text: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: String,
}

#[derive(Clone)]
struct OneShotApproval {
    request_id: String,
    command: String,
    turn: OneShotTurnSpec,
}

#[derive(Clone)]
struct CodexAppApproval {
    rpc_id: serde_json::Value,
}

struct CodexAppServerShared {
    app: AppHandle,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    thread_id: Mutex<Option<String>>,
    init_emitted: Mutex<bool>,
    current_turn_id: Mutex<Option<String>>,
    pending_approvals: Mutex<HashMap<String, CodexAppApproval>>,
    streaming_agent_items: Mutex<HashSet<String>>,
    responses: Mutex<HashMap<String, serde_json::Value>>,
    response_cv: Condvar,
    next_request_id: AtomicU64,
    latest_usage: Mutex<Option<UsageSummary>>,
}

struct ChatMeta {
    agent: String,
    project_key: String,
    cwd: String,
    session_id: Mutex<Option<String>>,
    title: Mutex<String>,
    permission_mode: String,
    model: Option<String>,
    effort: Option<String>,
    process_model: String,
    messages: Mutex<Vec<crate::types::Msg>>,
    turn_started_at_ms: Mutex<Option<u64>>,
}

type ChatEntry = (Arc<ChatHandle>, Arc<ChatMeta>);

static CHATS: OnceLock<Mutex<HashMap<u64, ChatEntry>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn map() -> &'static Mutex<HashMap<u64, ChatEntry>> {
    CHATS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_iso8601_utc() -> String {
    let ms = now_ms();
    crate::util::format_iso8601_utc((ms / 1000) as i64, (ms % 1000) as u32)
}

fn remember_msg(meta: &ChatMeta, msg: &crate::types::Msg) {
    if let Ok(mut messages) = meta.messages.lock() {
        let mut msg = msg.clone();
        if msg.timestamp.is_none() {
            msg.timestamp = Some(now_iso8601_utc());
        }
        messages.push(msg);
    }
}

fn remember_user_input(meta: &ChatMeta, text: &str, images: &[ChatImageInput]) {
    if text.trim().is_empty() && images.is_empty() {
        return;
    }
    let (file_blocks, body) = crate::agents::codex::extract_codex_files_pub(text);
    let mut blocks: Vec<crate::types::Block> = Vec::new();
    for img in images {
        blocks.push(crate::types::Block {
            kind: "image".to_string(),
            image_src: Some(format!("data:{};base64,{}", img.media_type, img.data)),
            ..Default::default()
        });
    }
    blocks.extend(file_blocks);
    if !body.trim().is_empty() {
        blocks.push(crate::util::text_block("text", &body));
    }
    if blocks.is_empty() && images.is_empty() {
        blocks.push(crate::util::text_block("text", text));
    }
    let mut msg = crate::types::Msg {
        uuid: None,
        role: "user".to_string(),
        timestamp: None,
        model: None,
        sidechain: false,
        blocks,
        meta_kind: None,
    };
    crate::util::post_process_session_msgs(std::slice::from_mut(&mut msg));
    remember_msg(meta, &msg);
}

fn mark_turn_started(meta: &ChatMeta) {
    if let Ok(mut started) = meta.turn_started_at_ms.lock() {
        *started = Some(now_ms());
    }
}

fn mark_turn_finished(meta: &ChatMeta) {
    if let Ok(mut started) = meta.turn_started_at_ms.lock() {
        *started = None;
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InitPayload {
    chat_id: u64,
    session_id: Option<String>,
    /// Claude init 的 apiKeySource：前端据此判断是否隐藏 5h/周限额角标（见 ChatEvent::Init）。
    api_key_source: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ResultPayload {
    chat_id: u64,
    ok: bool,
    usage: Option<UsageSummary>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StderrPayload {
    chat_id: u64,
    line: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ExitPayload {
    chat_id: u64,
    code: i32,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PermissionPayload {
    chat_id: u64,
    request: crate::types::ChatPermissionRequest,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct QuestionPayload {
    chat_id: u64,
    request: crate::types::ChatQuestionRequest,
}

/// 按 OS 组装管道子进程命令。与 `pty.rs::build_shell_command` 同款 PATH 策略，只是
/// 改用 `std::process::Command` + 三路管道（无 PTY）。
///
/// `use_reclaude`：用 reclaude 做进程包装器（`reclaude claude --print ...`），
/// 走 reclaude 守护进程的鉴权 + 代理链路。与 IDE 插件的 "Claude Process Wrapper" 同理。
#[cfg(unix)]
fn build_piped_command(
    cwd: &str,
    command: &AgentCommand,
    use_reclaude: bool,
) -> std::process::Command {
    #[cfg(target_os = "macos")]
    const DEFAULT_SHELL: &str = "/bin/zsh";
    #[cfg(not(target_os = "macos"))]
    const DEFAULT_SHELL: &str = "/bin/bash";

    let shell = std::env::var("SHELL").unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let cli = if use_reclaude {
        format!("'reclaude' {}", command.to_posix_shell())
    } else {
        command.to_posix_shell()
    };
    let inner = format!("cd {} && {}", crate::agent_command::posix_quote(cwd), cli);
    let mut cmd = std::process::Command::new(&shell);
    cmd.arg("-l").arg("-i").arg("-c").arg(&inner);
    cmd.env_remove("npm_config_prefix");
    cmd.current_dir(cwd);
    cmd
}

#[cfg(windows)]
fn build_piped_command(
    cwd: &str,
    command: &AgentCommand,
    use_reclaude: bool,
) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.arg("-NoLogo")
        .arg("-Command")
        .arg(crate::agent_command::powershell_set_location_and_run(
            cwd,
            command,
            use_reclaude,
        ));
    cmd.current_dir(cwd);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(unix)]
fn command_exists_in_login_shell(cwd: &str, program: &str) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    const DEFAULT_SHELL: &str = "/bin/zsh";
    #[cfg(not(target_os = "macos"))]
    const DEFAULT_SHELL: &str = "/bin/bash";

    let shell = std::env::var("SHELL").unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let check = format!(
        "cd {} && command -v {} >/dev/null 2>&1",
        crate::agent_command::posix_quote(cwd),
        crate::agent_command::posix_quote(program),
    );
    let status = std::process::Command::new(shell)
        .arg("-l")
        .arg("-i")
        .arg("-c")
        .arg(check)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to check {program} in login shell: {e}"))?;
    Ok(status.success())
}

#[cfg(windows)]
fn command_exists_in_login_shell(_cwd: &str, program: &str) -> Result<bool, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // 曾用裸 `where.exe <program>` 纯文件搜索。但 MSI（WiX advertised-shortcut）启动的进程
    // **穿不过目录符号链接**：nvm 的 codex 装在 NVM_SYMLINK（如 D:\nvm\nodejs，指向真实版本目录
    // 的 symlink）下，where.exe 走继承的、看不穿 symlink 的 PATH，于是 codex 明明装了也判"找不到"
    // → ensure_codex_cli_available 报错 → GUI chat 直接 ended（dev 从正常终端上下文起、能穿透
    // 所以正常，只有打包版复现）。改用与实际 spawn 同一套 powershell_refresh_path()（展开注册表
    // PATH + 把符号链接目录解析成真实目录），再 Get-Command 查。Get-Command 只解析命令位置、
    // 不执行 codex，不触发 nvm shim 的非终端检测；-ExecutionPolicy Bypass + CREATE_NO_WINDOW
    // 与 spawn 侧保持一致、无窗口闪。
    let check = format!(
        "{}; if (Get-Command {} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}",
        crate::agent_command::powershell_refresh_path(),
        crate::agent_command::powershell_quote(program),
    );
    let status = std::process::Command::new(crate::agent_command::windows_powershell_exe())
        .arg("-NoLogo")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(&check)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("Failed to check {program} via powershell: {e}"))?;
    Ok(status.success())
}

fn ensure_codex_cli_available(cwd: &str) -> Result<(), String> {
    if command_exists_in_login_shell(cwd, "codex")? {
        return Ok(());
    }
    Err(
        "Codex CLI was not found in PATH. Install Codex or make sure your login shell can run `codex`."
            .to_string(),
    )
}

fn read_reclaude_port(path: &std::path::Path) -> Option<u16> {
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let running = v.pointer("/daemon/running")?.as_bool()?;
    if !running {
        return None;
    }
    v.pointer("/daemon/port")?.as_u64().map(|p| p as u16)
}

pub fn reclaude_info() -> crate::types::ReclaudeInfo {
    let base = crate::util::home().join(".reclaude");
    if !base.is_dir() {
        return crate::types::ReclaudeInfo {
            installed: false,
            daemon_running: false,
            daemon_port: None,
        };
    }
    let state_path = base.join("state.json");
    let port = read_reclaude_port(&state_path);
    crate::types::ReclaudeInfo {
        installed: true,
        daemon_running: port.is_some(),
        daemon_port: port,
    }
}

/// 起一个 chat「会话」。`session_id` 给出时续聊既有会话；否则新开。返回内部 chat id。
/// 按该 agent 的 [`ChatProcessModel`] 选驱动路径：
///   - LongLivedStdin：spawn 一个长驻进程 + reader/stderr/waiter 线程（现状）。
///   - OneShotResume：**不 spawn**，只登记会话；首条 `send` 才起「这一轮」的进程。
#[allow(clippy::too_many_arguments)]
pub fn start(
    app: AppHandle,
    agent: String,
    project_key: String,
    cwd: String,
    session_id: Option<String>,
    permission_mode: String,
    model: Option<String>,
    effort: Option<String>,
    fork: bool,
    ephemeral: bool,
    use_reclaude: bool,
    preload_messages: Option<Vec<crate::types::Msg>>,
    title: Option<String>,
) -> Result<u64, String> {
    if !std::path::Path::new(&cwd).is_dir() {
        return Err("项目目录已不存在，无法启动聊天".into());
    }
    // reclaude 只适用于 Claude CLI；其他 agent（Codex 等）直接运行自己的 CLI。
    let use_reclaude = use_reclaude && agent == "claude";
    let source = agents::source(&agent)?;
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let pm_str = source.chat_process_model().as_str();
    let meta = Arc::new(ChatMeta {
        agent: agent.clone(),
        project_key,
        cwd: cwd.clone(),
        session_id: Mutex::new(session_id.clone()),
        title: Mutex::new(title.unwrap_or_default()),
        permission_mode: permission_mode.clone(),
        model: model.clone(),
        effort: effort.clone(),
        process_model: pm_str.to_string(),
        messages: Mutex::new(preload_messages.unwrap_or_default()),
        turn_started_at_ms: Mutex::new(None),
    });

    match source.chat_process_model() {
        ChatProcessModel::LongLivedStdin => {
            let command = source
                .chat_command(
                    session_id.as_deref(),
                    &permission_mode,
                    model.as_deref(),
                    effort.as_deref(),
                    fork,
                )
                .ok_or_else(|| format!("{agent} 暂不支持 GUI 聊天模式"))?;

            let mut cmd = build_piped_command(&cwd, &command, use_reclaude);
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
            let stdin = child.stdin.take().ok_or("failed to capture stdin")?;
            let stdout = child.stdout.take().ok_or("failed to capture stdout")?;
            let stderr = child.stderr.take().ok_or("failed to capture stderr")?;

            let handle = Arc::new(ChatHandle::LongLived {
                agent: agent.clone(),
                stdin: Mutex::new(stdin),
                child: Mutex::new(child),
            });
            map()
                .lock()
                .map_err(|e| e.to_string())?
                .insert(id, (handle, meta.clone()));

            let meta_for_reader = meta;
            let app_for_reader = app.clone();
            let agent_for_reader = agent.clone();
            thread::spawn(move || {
                reader_loop(
                    app_for_reader,
                    id,
                    agent_for_reader,
                    stdout,
                    meta_for_reader,
                )
            });
            spawn_stderr_pump(app.clone(), id, stderr);
            let app_for_wait = app.clone();
            thread::spawn(move || waiter_loop(app_for_wait, id));
        }
        ChatProcessModel::OneShotResume => {
            if agent == "codex" {
                let handle = start_codex_app_server(
                    app.clone(),
                    id,
                    meta.clone(),
                    agent.clone(),
                    cwd.clone(),
                    session_id.clone(),
                    &permission_mode,
                    model.as_deref(),
                    effort.as_deref(),
                    fork,
                    ephemeral,
                )?;
                if let ChatHandle::CodexAppServer { shared } = &*handle {
                    if let Some(thread_id) = shared.thread_id.lock().ok().and_then(|g| g.clone()) {
                        if let Ok(mut g) = meta.session_id.lock() {
                            *g = Some(thread_id);
                        }
                    }
                }
                map()
                    .lock()
                    .map_err(|e| e.to_string())?
                    .insert(id, (handle, meta));
                return Ok(id);
            }
            if source
                .chat_turn_command(session_id.as_deref(), "", &permission_mode, None, None)
                .is_none()
            {
                return Err(format!("{agent} 暂不支持 GUI 聊天模式"));
            }
            let handle = Arc::new(ChatHandle::OneShot {
                app: app.clone(),
                agent: agent.clone(),
                cwd: cwd.clone(),
                session_id: Mutex::new(session_id),
                current: Mutex::new(None),
                pending_approval: Mutex::new(None),
                approved_command_prefixes: Mutex::new(Vec::new()),
                use_reclaude,
            });
            map()
                .lock()
                .map_err(|e| e.to_string())?
                .insert(id, (handle, meta));
        }
    }

    Ok(id)
}

/// stderr 诊断行透传线程 —— 两条路径共用（长驻进程 / 每轮进程都有 stderr）。
fn spawn_stderr_pump(app: AppHandle, id: u64, stderr: std::process::ChildStderr) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if app
                        .emit(
                            "agent-chat://stderr",
                            StderrPayload {
                                chat_id: id,
                                line: trimmed,
                            },
                        )
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
}

fn rpc_id_key(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(s) => s.clone(),
        _ => id.to_string(),
    }
}

fn codex_next_rpc_id(shared: &CodexAppServerShared) -> serde_json::Value {
    serde_json::Value::Number(shared.next_request_id.fetch_add(1, Ordering::SeqCst).into())
}

fn codex_write_rpc(shared: &CodexAppServerShared, msg: serde_json::Value) -> Result<(), String> {
    let mut stdin = shared.stdin.lock().map_err(|e| e.to_string())?;
    stdin
        .write_all(msg.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    stdin.write_all(b"\n").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())
}

fn codex_wait_response(
    shared: &CodexAppServerShared,
    id: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let key = rpc_id_key(id);
    let mut responses = shared.responses.lock().map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    loop {
        if let Some(response) = responses.remove(&key) {
            if let Some(error) = response.get("error") {
                return Err(format!("codex app-server error: {error}"));
            }
            return Ok(response
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null));
        }
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return Err(format!(
                "Timed out waiting for codex app-server response: {key}"
            ));
        }
        let remaining = timeout - elapsed;
        let (next, wait) = shared
            .response_cv
            .wait_timeout(responses, remaining)
            .map_err(|e| e.to_string())?;
        responses = next;
        if wait.timed_out() {
            return Err(format!(
                "Timed out waiting for codex app-server response: {key}"
            ));
        }
    }
}

fn codex_thread_permission_overrides(
    permission_mode: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut params = serde_json::Map::new();
    match permission_mode {
        "ask" | "plan" | "approve" | "default" | "acceptEdits" => {
            params.insert("approvalPolicy".into(), serde_json::json!("on-request"));
            params.insert("approvalsReviewer".into(), serde_json::json!("user"));
            params.insert("sandbox".into(), serde_json::json!("workspace-write"));
        }
        "fullAccess" | "bypassPermissions" => {
            params.insert("approvalPolicy".into(), serde_json::json!("never"));
            params.insert("sandbox".into(), serde_json::json!("danger-full-access"));
        }
        // custom: leave config.toml in charge.
        _ => {}
    }
    params
}

fn codex_turn_permission_overrides(
    permission_mode: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut params = serde_json::Map::new();
    match permission_mode {
        "ask" | "plan" | "approve" | "default" | "acceptEdits" => {
            params.insert("approvalPolicy".into(), serde_json::json!("on-request"));
            params.insert("approvalsReviewer".into(), serde_json::json!("user"));
            params.insert(
                "sandboxPolicy".into(),
                serde_json::json!({ "type": "workspaceWrite" }),
            );
        }
        "fullAccess" | "bypassPermissions" => {
            params.insert("approvalPolicy".into(), serde_json::json!("never"));
            params.insert(
                "sandboxPolicy".into(),
                serde_json::json!({ "type": "dangerFullAccess" }),
            );
        }
        // custom: leave config.toml in charge.
        _ => {}
    }
    params
}

fn codex_turn_params(
    thread_id: &str,
    text: &str,
    permission_mode: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> serde_json::Value {
    let mut params = codex_turn_permission_overrides(permission_mode);
    params.insert("threadId".into(), serde_json::json!(thread_id));
    params.insert(
        "input".into(),
        serde_json::json!([{
            "type": "text",
            "text": text,
            "textElements": [],
        }]),
    );
    if let Some(model) = model {
        params.insert("model".into(), serde_json::json!(model));
    }
    if let Some(effort) = effort {
        params.insert("effort".into(), serde_json::json!(effort));
    }
    serde_json::Value::Object(params)
}

fn codex_thread_params(
    cwd: &str,
    permission_mode: &str,
    model: Option<&str>,
    effort: Option<&str>,
    ephemeral: bool,
) -> serde_json::Value {
    let mut params = codex_thread_permission_overrides(permission_mode);
    params.insert("cwd".into(), serde_json::json!(cwd));
    if let Some(model) = model {
        params.insert("model".into(), serde_json::json!(model));
    }
    if let Some(effort) = effort {
        params.insert("effort".into(), serde_json::json!(effort));
    }
    if ephemeral {
        params.insert("ephemeral".into(), serde_json::json!(true));
    }
    serde_json::Value::Object(params)
}

/// `thread/fork` 的参数形状和 `thread/start` 略有不同：它可以直接接收 threadId，
/// 也原生支持 `ephemeral: true`，因此 Codex `/side` 无需在关闭时删除磁盘 transcript。
fn codex_thread_fork_params(
    thread_id: &str,
    cwd: &str,
    permission_mode: &str,
    model: Option<&str>,
    ephemeral: bool,
) -> serde_json::Value {
    let mut params = codex_thread_permission_overrides(permission_mode);
    params.insert("threadId".into(), serde_json::json!(thread_id));
    params.insert("cwd".into(), serde_json::json!(cwd));
    // side 面板只渲染新分支产生的消息，不需要把源 thread 的全部 turns 回传一遍。
    params.insert("excludeTurns".into(), serde_json::json!(true));
    if let Some(model) = model {
        params.insert("model".into(), serde_json::json!(model));
    }
    if ephemeral {
        params.insert("ephemeral".into(), serde_json::json!(true));
    }
    serde_json::Value::Object(params)
}

#[cfg(test)]
mod codex_side_tests {
    use super::{codex_thread_fork_params, codex_thread_params};

    #[test]
    fn fork_params_preserve_source_and_request_ephemeral_thread() {
        let params = codex_thread_fork_params(
            "source-thread",
            "/workspace/app",
            "approve",
            Some("gpt-5.4"),
            true,
        );

        assert_eq!(params["threadId"], "source-thread");
        assert_eq!(params["cwd"], "/workspace/app");
        assert_eq!(params["model"], "gpt-5.4");
        assert_eq!(params["ephemeral"], true);
        assert_eq!(params["excludeTurns"], true);
        assert_eq!(params["sandbox"], "workspace-write");
    }

    #[test]
    fn fresh_side_thread_can_be_ephemeral_without_a_fork_source() {
        let params = codex_thread_params(
            "/workspace/app",
            "fullAccess",
            None,
            Some("high"),
            true,
        );

        assert_eq!(params["cwd"], "/workspace/app");
        assert_eq!(params["ephemeral"], true);
        assert_eq!(params["sandbox"], "danger-full-access");
    }
}

fn usage_from_app_server_breakdown(v: &serde_json::Value) -> UsageSummary {
    let input = v
        .get("inputTokens")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as u64;
    let cached = v
        .get("cachedInputTokens")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as u64;
    let output = v
        .get("outputTokens")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as u64;
    let reasoning = v
        .get("reasoningOutputTokens")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as u64;
    UsageSummary {
        input_tokens: input,
        output_tokens: output,
        cache_creation_input_tokens: 0,
        cache_creation_1h_input_tokens: 0,
        cache_read_input_tokens: cached,
        reasoning_output_tokens: reasoning,
        total: input + output,
    }
}

#[allow(clippy::too_many_arguments)]
fn start_codex_app_server(
    app: AppHandle,
    id: u64,
    meta: Arc<ChatMeta>,
    _agent: String,
    cwd: String,
    session_id: Option<String>,
    permission_mode: &str,
    model: Option<&str>,
    effort: Option<&str>,
    fork: bool,
    ephemeral: bool,
) -> Result<Arc<ChatHandle>, String> {
    ensure_codex_cli_available(&cwd)?;
    let command = AgentCommand::new("codex").arg("app-server");
    let mut cmd = build_piped_command(&cwd, &command, false);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let stdin = child.stdin.take().ok_or("failed to capture stdin")?;
    let stdout = child.stdout.take().ok_or("failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("failed to capture stderr")?;

    let shared = Arc::new(CodexAppServerShared {
        app: app.clone(),
        stdin: Mutex::new(stdin),
        child: Mutex::new(child),
        thread_id: Mutex::new(None),
        init_emitted: Mutex::new(false),
        current_turn_id: Mutex::new(None),
        pending_approvals: Mutex::new(HashMap::new()),
        streaming_agent_items: Mutex::new(HashSet::new()),
        responses: Mutex::new(HashMap::new()),
        response_cv: Condvar::new(),
        next_request_id: AtomicU64::new(1),
        latest_usage: Mutex::new(None),
    });

    spawn_stderr_pump(app.clone(), id, stderr);
    let app_for_reader = app.clone();
    let shared_for_reader = shared.clone();
    thread::spawn(move || {
        codex_app_server_reader(app_for_reader, id, stdout, shared_for_reader, meta)
    });

    let init_id = codex_next_rpc_id(&shared);
    codex_write_rpc(
        &shared,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "cc-sessions-viewer",
                    "title": "Claude Session Viewer",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            },
        }),
    )?;
    let _ = codex_wait_response(&shared, &init_id, Duration::from_secs(10))?;
    codex_write_rpc(
        &shared,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {},
        }),
    )?;

    let thread_id_rpc = codex_next_rpc_id(&shared);
    let (method, params) = if fork {
        let source_id = session_id
            .as_deref()
            .ok_or_else(|| "Codex side conversation requires a source thread".to_string())?;
        (
            "thread/fork",
            codex_thread_fork_params(
                source_id,
                &cwd,
                permission_mode,
                model,
                ephemeral,
            ),
        )
    } else if let Some(session_id) = session_id.as_ref() {
        let mut params = codex_thread_permission_overrides(permission_mode);
        params.insert("threadId".into(), serde_json::json!(session_id));
        params.insert("cwd".into(), serde_json::json!(cwd));
        if let Some(model) = model {
            params.insert("model".into(), serde_json::json!(model));
        }
        if let Some(effort) = effort {
            params.insert("effort".into(), serde_json::json!(effort));
        }
        ("thread/resume", serde_json::Value::Object(params))
    } else {
        (
            "thread/start",
            codex_thread_params(&cwd, permission_mode, model, effort, ephemeral),
        )
    };
    codex_write_rpc(
        &shared,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": thread_id_rpc,
            "method": method,
            "params": params,
        }),
    )?;
    let result = codex_wait_response(&shared, &thread_id_rpc, Duration::from_secs(15))?;
    let thread_id = result
        .pointer("/thread/id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(session_id)
        .ok_or_else(|| "codex app-server did not return a thread id".to_string())?;
    if let Ok(mut g) = shared.thread_id.lock() {
        *g = Some(thread_id.clone());
    }

    Ok(Arc::new(ChatHandle::CodexAppServer { shared }))
}

fn emit_codex_init_if_needed(id: u64, shared: &CodexAppServerShared) {
    let Some(thread_id) = shared.thread_id.lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    let Ok(mut emitted) = shared.init_emitted.lock() else {
        return;
    };
    if *emitted {
        return;
    }
    *emitted = true;
    let _ = shared.app.emit(
        "agent-chat://init",
        InitPayload {
            chat_id: id,
            session_id: Some(thread_id),
            api_key_source: None,
        },
    );
}

fn emit_codex_agent_delta(
    app: &AppHandle,
    id: u64,
    shared: &CodexAppServerShared,
    params: &serde_json::Value,
) {
    let Some(item_id) = params
        .get("itemId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let Some(text) = params.get("delta").and_then(serde_json::Value::as_str) else {
        return;
    };
    let is_first = shared
        .streaming_agent_items
        .lock()
        .map(|mut seen| seen.insert(item_id))
        .unwrap_or(false);
    if is_first {
        let _ = app.emit(
            "agent-chat://delta",
            DeltaPayload {
                chat_id: id,
                delta: crate::types::ChatDelta {
                    index: 0,
                    phase: "start".to_string(),
                    kind: Some("text".to_string()),
                    text: None,
                },
            },
        );
    }
    let _ = app.emit(
        "agent-chat://delta",
        DeltaPayload {
            chat_id: id,
            delta: crate::types::ChatDelta {
                index: 0,
                phase: "delta".to_string(),
                kind: Some("text".to_string()),
                text: Some(text.to_string()),
            },
        },
    );
}

fn codex_item_to_msg(item: &serde_json::Value) -> Option<crate::types::Msg> {
    match item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
    {
        "agentMessage" => {
            let text = item
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if text.trim().is_empty() {
                return None;
            }
            Some(crate::util::simple_msg(
                "assistant",
                None,
                crate::util::text_block("text", text),
            ))
        }
        "commandExecution" => {
            let command = item
                .get("command")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if command.trim().is_empty() {
                return None;
            }
            let output = item
                .get("aggregatedOutput")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let status = item
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let exit_code = item.get("exitCode").and_then(serde_json::Value::as_i64);
            let tool_id = item
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let mut blocks = vec![crate::types::Block {
                kind: "tool_use".to_string(),
                tool_name: Some("shell".to_string()),
                tool_input: Some(command.to_string()),
                tool_id: tool_id.clone(),
                ..Default::default()
            }];
            if !output.trim().is_empty() || exit_code.is_some() || status == "declined" {
                blocks.push(crate::types::Block {
                    kind: "tool_result".to_string(),
                    text: Some(output.to_string()),
                    tool_id,
                    is_error: status == "failed"
                        || status == "declined"
                        || exit_code.is_some_and(|c| c != 0),
                    ..Default::default()
                });
            }
            Some(crate::types::Msg {
                uuid: None,
                role: "assistant".to_string(),
                timestamp: None,
                model: None,
                sidechain: false,
                blocks,
                meta_kind: None,
            })
        }
        "reasoning" | "userMessage" | "plan" => None,
        other if !other.is_empty() => {
            let summary = serde_json::to_string(item).unwrap_or_default();
            Some(crate::util::simple_msg(
                "assistant",
                None,
                crate::types::Block {
                    kind: "tool_use".to_string(),
                    tool_name: Some(other.to_string()),
                    tool_input: Some(summary.chars().take(1200).collect()),
                    ..Default::default()
                },
            ))
        }
        _ => None,
    }
}

fn emit_codex_permission_request(
    app: &AppHandle,
    id: u64,
    shared: &Arc<CodexAppServerShared>,
    rpc_id: serde_json::Value,
    params: &serde_json::Value,
) -> bool {
    let request_id = rpc_id_key(&rpc_id);
    if let Ok(mut pending) = shared.pending_approvals.lock() {
        pending.insert(
            request_id.clone(),
            CodexAppApproval {
                rpc_id: rpc_id.clone(),
            },
        );
    }
    let command = params
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let reason = params
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Codex needs approval before running this command.")
        .to_string();
    let environment = params
        .get("environmentId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("local")
        .to_string();
    let has_accept_for_session = params
        .get("availableDecisions")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str() == Some("acceptForSession"))
        });
    let permission_suggestions = has_accept_for_session.then(|| {
        serde_json::json!([
            {
                "type": "codexAcceptForSession",
                "behavior": "allow"
            }
        ])
    });
    let request = crate::types::ChatPermissionRequest {
        request_id,
        tool_name: "shell".to_string(),
        input: serde_json::json!({
            "command": command,
            "cwd": params.get("cwd").cloned().unwrap_or(serde_json::Value::Null),
            "environment": environment,
            "reason": reason,
        }),
        description: Some("Codex is requesting permission to run a command.".to_string()),
        permission_suggestions,
    };
    app.emit(
        "agent-chat://permission",
        PermissionPayload {
            chat_id: id,
            request,
        },
    )
    .is_ok()
}

fn codex_app_server_reader(
    app: AppHandle,
    id: u64,
    stdout: ChildStdout,
    shared: Arc<CodexAppServerShared>,
    meta: Arc<ChatMeta>,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(rpc_id) = value.get("id") {
            if value.get("result").is_some() || value.get("error").is_some() {
                if let Ok(mut responses) = shared.responses.lock() {
                    responses.insert(rpc_id_key(rpc_id), value.clone());
                    shared.response_cv.notify_all();
                }
                continue;
            }
        }
        let Some(method) = value.get("method").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let params = value
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        match method {
            "item/commandExecution/requestApproval" => {
                if let Some(rpc_id) = value.get("id").cloned() {
                    let _ = emit_codex_permission_request(&app, id, &shared, rpc_id, &params);
                }
            }
            "item/agentMessage/delta" => {
                emit_codex_agent_delta(&app, id, &shared, &params);
            }
            "item/completed" => {
                if let Some(item_id) = params
                    .pointer("/item/id")
                    .and_then(serde_json::Value::as_str)
                {
                    let was_streaming = shared
                        .streaming_agent_items
                        .lock()
                        .map(|mut seen| seen.remove(item_id))
                        .unwrap_or(false);
                    if was_streaming {
                        let _ = app.emit(
                            "agent-chat://delta",
                            DeltaPayload {
                                chat_id: id,
                                delta: crate::types::ChatDelta {
                                    index: 0,
                                    phase: "stop".to_string(),
                                    kind: Some("text".to_string()),
                                    text: None,
                                },
                            },
                        );
                    }
                }
                if let Some(msg) = params.get("item").and_then(codex_item_to_msg) {
                    remember_msg(&meta, &msg);
                    let _ = app.emit("agent-chat://event", EventPayload { chat_id: id, msg });
                }
            }
            "thread/tokenUsage/updated" => {
                if let Some(last) = params.pointer("/tokenUsage/last") {
                    if let Ok(mut latest) = shared.latest_usage.lock() {
                        *latest = Some(usage_from_app_server_breakdown(last));
                    }
                }
            }
            "turn/started" => {
                if let Some(turn_id) = params
                    .pointer("/turn/id")
                    .and_then(serde_json::Value::as_str)
                {
                    if let Ok(mut g) = shared.current_turn_id.lock() {
                        *g = Some(turn_id.to_string());
                    }
                }
            }
            "turn/completed" => {
                let ok = params
                    .pointer("/turn/status")
                    .and_then(serde_json::Value::as_str)
                    .map(|s| s == "completed")
                    .unwrap_or(true);
                let usage = shared.latest_usage.lock().ok().and_then(|g| *g);
                let _ = app.emit(
                    "agent-chat://result",
                    ResultPayload {
                        chat_id: id,
                        ok,
                        usage,
                    },
                );
                mark_turn_finished(&meta);
                if let Ok(mut g) = shared.current_turn_id.lock() {
                    *g = None;
                }
            }
            "error" => {
                let message = params
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Codex app-server error");
                let msg = crate::util::simple_msg(
                    "assistant",
                    None,
                    crate::util::text_block("system_event", message),
                );
                remember_msg(&meta, &msg);
                let _ = app.emit("agent-chat://event", EventPayload { chat_id: id, msg });
                let _ = app.emit(
                    "agent-chat://result",
                    ResultPayload {
                        chat_id: id,
                        ok: false,
                        usage: None,
                    },
                );
                mark_turn_finished(&meta);
            }
            _ => {}
        }
    }
    let _ = app.emit(
        "agent-chat://exit",
        ExitPayload {
            chat_id: id,
            code: -1,
        },
    );
}

fn reader_loop(
    app: AppHandle,
    id: u64,
    agent: String,
    stdout: std::process::ChildStdout,
    meta: Arc<ChatMeta>,
) {
    let Ok(source) = agents::source(&agent) else {
        return;
    };
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let emit_ok = match source.parse_chat_line(&line) {
            ChatEvent::Message(msg) => {
                remember_msg(&meta, &msg);
                app.emit("agent-chat://event", EventPayload { chat_id: id, msg })
                    .is_ok()
            }
            ChatEvent::Init {
                session_id,
                api_key_source,
            } => {
                if let Some(s) = session_id.as_ref() {
                    if let Ok(mut g) = meta.session_id.lock() {
                        *g = Some(s.clone());
                    }
                }
                app.emit(
                    "agent-chat://init",
                    InitPayload {
                        chat_id: id,
                        session_id,
                        api_key_source,
                    },
                )
                .is_ok()
            }
            ChatEvent::Result { ok, usage } => {
                mark_turn_finished(&meta);
                app.emit(
                    "agent-chat://result",
                    ResultPayload {
                        chat_id: id,
                        ok,
                        usage,
                    },
                )
                .is_ok()
            }
            ChatEvent::Delta(delta) => app
                .emit("agent-chat://delta", DeltaPayload { chat_id: id, delta })
                .is_ok(),
            ChatEvent::Permission(request) => app
                .emit(
                    "agent-chat://permission",
                    PermissionPayload {
                        chat_id: id,
                        request,
                    },
                )
                .is_ok(),
            ChatEvent::Question(request) => app
                .emit(
                    "agent-chat://question",
                    QuestionPayload {
                        chat_id: id,
                        request,
                    },
                )
                .is_ok(),
            ChatEvent::Ignore => true,
        };
        if !emit_ok {
            break;
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EventPayload {
    chat_id: u64,
    msg: crate::types::Msg,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeltaPayload {
    chat_id: u64,
    delta: crate::types::ChatDelta,
}

fn waiter_loop(app: AppHandle, id: u64) {
    loop {
        let res = {
            let arc = match map()
                .lock()
                .ok()
                .and_then(|m| m.get(&id).map(|(h, _)| h.clone()))
            {
                Some(a) => a,
                None => return,
            };
            let ChatHandle::LongLived { child, .. } = &*arc else {
                return;
            };
            let mut child = match child.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            child.try_wait()
        };
        match res {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);
                let _ = app.emit("agent-chat://exit", ExitPayload { chat_id: id, code });
                if let Ok(mut m) = map().lock() {
                    m.remove(&id);
                }
                return;
            }
            Ok(None) => thread::sleep(Duration::from_millis(150)),
            Err(_) => {
                let _ = app.emit(
                    "agent-chat://exit",
                    ExitPayload {
                        chat_id: id,
                        code: -1,
                    },
                );
                if let Ok(mut m) = map().lock() {
                    m.remove(&id);
                }
                return;
            }
        }
    }
}

/// 发送一条用户消息（含可选图片附件）。按进程模型分两条路：
///   - LongLived：用 `chat_encode_input` 编一行写进长驻进程 stdin。`model`/`effort`/
///     `permission_mode` 在 start 时已定型，此处忽略（切换走 restart-with-resume）。
///   - OneShot：spawn 一个 `chat_turn_command(...)` 进程跑这一轮 —— 三者每轮自带，
///     故模型 / effort / 权限切换免费即时生效（下一轮带新 flag）。
pub fn send(
    id: u64,
    text: &str,
    images: &[ChatImageInput],
    model: Option<&str>,
    effort: Option<&str>,
    permission_mode: &str,
) -> Result<(), String> {
    let (arc, meta) = {
        let m = map().lock().map_err(|e| e.to_string())?;
        m.get(&id)
            .map(|(h, meta)| (h.clone(), meta.clone()))
            .ok_or_else(|| "chat not found".to_string())?
    };
    if text.is_empty() && images.is_empty() {
        return Ok(()); // 空消息不发。
    }
    remember_user_input(&meta, text, images);
    mark_turn_started(&meta);

    match &*arc {
        ChatHandle::LongLived { agent, stdin, .. } => {
            let source = agents::source(agent)?;
            let mut line = source.chat_encode_input(text, images);
            line.push('\n');
            let mut w = stdin.lock().map_err(|e| e.to_string())?;
            w.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            w.flush().map_err(|e| e.to_string())?;
            Ok(())
        }
        ChatHandle::OneShot {
            pending_approval, ..
        } => {
            if let Ok(mut pending) = pending_approval.lock() {
                *pending = None;
            }
            let spec = OneShotTurnSpec {
                text: text.to_string(),
                model: model.map(str::to_string),
                effort: effort.map(str::to_string),
                permission_mode: permission_mode.to_string(),
            };
            spawn_oneshot_turn(id, arc.clone(), spec)
        }
        ChatHandle::CodexAppServer { shared, .. } => {
            let thread_id = shared
                .thread_id
                .lock()
                .map_err(|e| e.to_string())?
                .clone()
                .ok_or_else(|| "codex app-server thread not initialized".to_string())?;
            emit_codex_init_if_needed(id, shared);
            let rpc_id = codex_next_rpc_id(shared);
            codex_write_rpc(
                shared,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": rpc_id,
                    "method": "turn/start",
                    "params": codex_turn_params(
                        &thread_id,
                        text,
                        permission_mode,
                        model,
                        effort,
                    ),
                }),
            )
        }
    }
}

fn spawn_oneshot_turn(id: u64, arc: Arc<ChatHandle>, spec: OneShotTurnSpec) -> Result<(), String> {
    let ChatHandle::OneShot {
        app,
        agent,
        cwd,
        session_id,
        current,
        use_reclaude,
        ..
    } = &*arc
    else {
        return Err("chat is not a one-shot session".into());
    };
    let source = agents::source(agent)?;
    let sid = session_id.lock().ok().and_then(|g| g.clone());
    let command = source
        .chat_turn_command(
            sid.as_deref(),
            &spec.text,
            &spec.permission_mode,
            spec.model.as_deref(),
            spec.effort.as_deref(),
        )
        .ok_or_else(|| format!("{agent} 暂不支持 GUI 聊天模式"))?;

    let mut cmd = build_piped_command(cwd, &command, *use_reclaude);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let stdout = child.stdout.take().ok_or("failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("failed to capture stderr")?;
    if let Ok(mut g) = current.lock() {
        *g = Some(child);
    }
    spawn_stderr_pump(app.clone(), id, stderr);
    let app_for_reader = app.clone();
    let agent_for_reader = agent.clone();
    let arc_for_reader = arc.clone();
    thread::spawn(move || {
        oneshot_turn_reader(
            app_for_reader,
            id,
            agent_for_reader,
            stdout,
            arc_for_reader,
            spec,
        )
    });
    Ok(())
}

fn permission_denial_command(msg: &crate::types::Msg) -> Option<String> {
    let mut command: Option<String> = None;
    let mut denied = false;
    for block in &msg.blocks {
        if block.kind == "tool_use" && block.tool_name.as_deref() == Some("shell") {
            if let Some(input) = block.tool_input.as_deref() {
                if !input.trim().is_empty() {
                    command = Some(input.to_string());
                }
            }
        }
        if block.kind == "tool_result" && block.is_error {
            let text = block.text.as_deref().unwrap_or("").to_ascii_lowercase();
            denied = text.contains("operation not permitted")
                || text.contains("permission denied")
                || text.contains("not permitted")
                || text.contains("sandbox");
        }
    }
    if denied {
        command
    } else {
        None
    }
}

fn kill_current_oneshot(arc: &Arc<ChatHandle>) {
    if let ChatHandle::OneShot { current, .. } = &**arc {
        if let Ok(mut g) = current.lock() {
            if let Some(child) = g.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

fn wait_and_clear_current_oneshot(arc: &Arc<ChatHandle>) {
    if let ChatHandle::OneShot { current, .. } = &**arc {
        if let Ok(mut g) = current.lock() {
            if let Some(mut c) = g.take() {
                let _ = c.wait();
            }
        }
    }
}

fn one_shot_request_id(id: u64) -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("codex-approval-{id}-{n}")
}

fn command_is_approved(arc: &Arc<ChatHandle>, command: &str) -> bool {
    let ChatHandle::OneShot {
        approved_command_prefixes,
        ..
    } = &**arc
    else {
        return false;
    };
    approved_command_prefixes
        .lock()
        .ok()
        .is_some_and(|prefixes| prefixes.iter().any(|p| command.starts_with(p)))
}

fn emit_one_shot_permission(
    app: &AppHandle,
    id: u64,
    arc: &Arc<ChatHandle>,
    command: String,
    spec: OneShotTurnSpec,
) -> bool {
    let request_id = one_shot_request_id(id);
    if let ChatHandle::OneShot {
        pending_approval, ..
    } = &**arc
    {
        if let Ok(mut pending) = pending_approval.lock() {
            *pending = Some(OneShotApproval {
                request_id: request_id.clone(),
                command: command.clone(),
                turn: spec,
            });
        }
    }
    let request = crate::types::ChatPermissionRequest {
        request_id,
        tool_name: "shell".to_string(),
        input: serde_json::json!({
            "command": command.clone(),
            "environment": "local",
            "reason": "The command was blocked by the current sandbox or filesystem permissions.",
        }),
        description: Some("The command needs elevated local permissions to continue.".to_string()),
        permission_suggestions: Some(serde_json::json!([
            {
                "type": "codexCommandPrefix",
                "behavior": "allow",
                "prefix": command,
            }
        ])),
    };
    app.emit(
        "agent-chat://permission",
        PermissionPayload {
            chat_id: id,
            request,
        },
    )
    .is_ok()
}

fn request_or_rerun_oneshot_permission(
    app: &AppHandle,
    id: u64,
    arc: &Arc<ChatHandle>,
    spec: &OneShotTurnSpec,
    command: String,
    waiting_for_permission: &mut bool,
    auto_rerun: &mut Option<OneShotTurnSpec>,
) {
    kill_current_oneshot(arc);
    let mut retry = spec.clone();
    retry.permission_mode = "fullAccess".to_string();
    if command_is_approved(arc, &command) {
        *auto_rerun = Some(retry);
    } else {
        *waiting_for_permission = emit_one_shot_permission(app, id, arc, command, retry);
    }
}

/// OneShot「这一轮」的 stdout reader：归一事件 → emit；`Init` 回填会话 session_id；
/// 进程退出（stdout EOF）时收尾 —— 没见过 `Result` 就补一条失败 Result，并清掉
/// `current`（该轮进程已结束）。**不** emit `exit`：one-shot 的「会话」要活到 stop。
fn oneshot_turn_reader(
    app: AppHandle,
    id: u64,
    agent: String,
    stdout: std::process::ChildStdout,
    arc: Arc<ChatHandle>,
    spec: OneShotTurnSpec,
) {
    let Ok(source) = agents::source(&agent) else {
        return;
    };
    let reader = BufReader::new(stdout);
    let mut saw_result = false;
    let mut waiting_for_permission = false;
    let mut auto_rerun: Option<OneShotTurnSpec> = None;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let emit_ok = match source.parse_chat_line(&line) {
            ChatEvent::Message(msg) => {
                let denied_command = if agent == "codex" {
                    permission_denial_command(&msg)
                } else {
                    None
                };
                let ok = app
                    .emit("agent-chat://event", EventPayload { chat_id: id, msg })
                    .is_ok();
                if ok {
                    if let Some(command) = denied_command {
                        request_or_rerun_oneshot_permission(
                            &app,
                            id,
                            &arc,
                            &spec,
                            command,
                            &mut waiting_for_permission,
                            &mut auto_rerun,
                        );
                        false
                    } else {
                        true
                    }
                } else {
                    false
                }
            }
            ChatEvent::Init {
                session_id,
                api_key_source,
            } => {
                if let (
                    Some(s),
                    ChatHandle::OneShot {
                        session_id: slot, ..
                    },
                ) = (session_id.as_ref(), &*arc)
                {
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(s.clone());
                    }
                }
                app.emit(
                    "agent-chat://init",
                    InitPayload {
                        chat_id: id,
                        session_id,
                        api_key_source,
                    },
                )
                .is_ok()
            }
            ChatEvent::Result { ok, usage } => {
                saw_result = true;
                app.emit(
                    "agent-chat://result",
                    ResultPayload {
                        chat_id: id,
                        ok,
                        usage,
                    },
                )
                .is_ok()
            }
            // OneShot agent（Codex）目前不产 Delta；保留分支以满足穷尽匹配。
            ChatEvent::Delta(delta) => app
                .emit("agent-chat://delta", DeltaPayload { chat_id: id, delta })
                .is_ok(),
            // 交互式权限请求 / 结构化提问只在长驻 stdin 模型（Claude）出现；OneShot 没有可
            // 回写的长驻 stdin，故这里不该收到 —— 忽略即可（保持穷尽匹配）。
            ChatEvent::Permission(_) | ChatEvent::Question(_) => true,
            ChatEvent::Ignore => true,
        };
        if !emit_ok {
            break;
        }
    }
    // 该轮进程已退出。
    wait_and_clear_current_oneshot(&arc);
    if let Some(retry) = auto_rerun {
        let _ = spawn_oneshot_turn(id, arc.clone(), retry);
        return;
    }
    if !saw_result && !waiting_for_permission {
        let _ = app.emit(
            "agent-chat://result",
            ResultPayload {
                chat_id: id,
                ok: false,
                usage: None,
            },
        );
    }
}

/// 结束一个 chat 会话：先把 entry 拿走（waiter 下一轮发现不见了就 return，
/// 不再 emit 奇怪的 exit），再 kill + wait 回收，避免僵尸。幂等。
pub fn stop(id: u64) -> Result<(), String> {
    let entry = {
        let mut m = map().lock().map_err(|e| e.to_string())?;
        m.remove(&id)
    };
    let Some((arc, _meta)) = entry else {
        return Ok(());
    };
    match &*arc {
        ChatHandle::LongLived { child, .. } => {
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        ChatHandle::OneShot { current, .. } => {
            // 杀掉当前在跑的那一轮（如果有）；没有在跑就只是从 map 摘除。
            if let Ok(mut g) = current.lock() {
                if let Some(mut c) = g.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
        }
        ChatHandle::CodexAppServer { shared, .. } => {
            if let Ok(mut child) = shared.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    Ok(())
}

/// 仅中断当前这轮生成，不结束 chat 会话本身。
/// Claude 长驻进程里这应等价于用户在 CLI 按一次 Esc：当前请求打断，但进程继续存活，下一条
/// 消息还能继续发。OneShot agent 没有长驻 stdin / 没有可复用会话进程，回退到 stop。
pub fn interrupt(id: u64) -> Result<(), String> {
    let arc = {
        let m = map().lock().map_err(|e| e.to_string())?;
        m.get(&id)
            .map(|(h, _)| h.clone())
            .ok_or_else(|| "chat not found".to_string())?
    };
    match &*arc {
        ChatHandle::LongLived { stdin, .. } => {
            let mut w = stdin.lock().map_err(|e| e.to_string())?;
            w.write_all(&[0x1b]).map_err(|e| e.to_string())?;
            w.flush().map_err(|e| e.to_string())?;
            Ok(())
        }
        ChatHandle::OneShot { .. } => stop(id),
        ChatHandle::CodexAppServer { shared, .. } => {
            let thread_id = shared.thread_id.lock().ok().and_then(|g| g.clone());
            let turn_id = shared.current_turn_id.lock().ok().and_then(|g| g.clone());
            match (thread_id, turn_id) {
                (Some(thread_id), Some(turn_id)) => {
                    let rpc_id = codex_next_rpc_id(shared);
                    codex_write_rpc(
                        shared,
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": rpc_id,
                            "method": "turn/interrupt",
                            "params": {
                                "threadId": thread_id,
                                "turnId": turn_id,
                            },
                        }),
                    )
                }
                _ => Ok(()),
            }
        }
    }
}

/// 回写一次控制协议决定（响应 `can_use_tool`，覆盖工具权限与 AskUserQuestion 两类请求）。
/// 把前端构造好的 `decision`（`{behavior:"allow",updatedInput,...}` /
/// `{behavior:"deny",message,interrupt}`）包进 `control_response`，写进长驻进程的同一条 stdin
/// （与用户消息、Esc 同管道）：
///   `{"type":"control_response","response":{"subtype":"success","request_id":<id>,"response":<decision>}}`
/// 只有长驻 stdin 模型（Claude）有这条回路；OneShot 不产生此类请求，调用即报错。
fn respond_control(id: u64, request_id: &str, decision: serde_json::Value) -> Result<(), String> {
    let arc = {
        let m = map().lock().map_err(|e| e.to_string())?;
        m.get(&id)
            .map(|(h, _)| h.clone())
            .ok_or_else(|| "chat not found".to_string())?
    };
    match &*arc {
        ChatHandle::LongLived { stdin, .. } => {
            let line = serde_json::json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": decision,
                }
            })
            .to_string();
            let mut w = stdin.lock().map_err(|e| e.to_string())?;
            w.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            w.write_all(b"\n").map_err(|e| e.to_string())?;
            w.flush().map_err(|e| e.to_string())?;
            Ok(())
        }
        ChatHandle::OneShot {
            app,
            pending_approval,
            approved_command_prefixes,
            ..
        } => {
            let pending = pending_approval
                .lock()
                .map_err(|e| e.to_string())?
                .take()
                .filter(|p| p.request_id == request_id);
            let Some(pending) = pending else {
                return Err("permission request not found".into());
            };
            match decision.get("behavior").and_then(|b| b.as_str()) {
                Some("allow") => {
                    if decision.get("updatedPermissions").is_some() {
                        if let Ok(mut prefixes) = approved_command_prefixes.lock() {
                            if !prefixes.iter().any(|p| p == &pending.command) {
                                prefixes.push(pending.command.clone());
                            }
                        }
                    }
                    spawn_oneshot_turn(id, arc.clone(), pending.turn)
                }
                Some("deny") => app
                    .emit(
                        "agent-chat://result",
                        ResultPayload {
                            chat_id: id,
                            ok: false,
                            usage: None,
                        },
                    )
                    .map_err(|e| e.to_string()),
                _ => Err("Invalid permission decision".into()),
            }
        }
        ChatHandle::CodexAppServer { shared, .. } => {
            let pending = shared
                .pending_approvals
                .lock()
                .map_err(|e| e.to_string())?
                .remove(request_id);
            let Some(pending) = pending else {
                return Err("permission request not found".into());
            };
            let response_decision = match decision.get("behavior").and_then(|b| b.as_str()) {
                Some("allow") if decision.get("updatedPermissions").is_some() => "acceptForSession",
                Some("allow") => "accept",
                Some("deny") => "decline",
                _ => return Err("Invalid permission decision".into()),
            };
            codex_write_rpc(
                shared,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": pending.rpc_id,
                    "result": {
                        "decision": response_decision,
                    },
                }),
            )
        }
    }
}

/// 回写一次交互式工具权限决定（响应 `agent-chat://permission`）。
pub fn respond_permission(
    id: u64,
    request_id: &str,
    decision: serde_json::Value,
) -> Result<(), String> {
    respond_control(id, request_id, decision)
}

/// 回写一次结构化提问的答案决定（响应 `agent-chat://question`）。`decision` 已由前端构造成
/// `{behavior:"allow",updatedInput:{questions,answers,response?}}`（作答）或
/// `{behavior:"deny",message,interrupt:false}`（取消，反馈给模型但不打断本轮）。
pub fn respond_question(
    id: u64,
    request_id: &str,
    decision: serde_json::Value,
) -> Result<(), String> {
    respond_control(id, request_id, decision)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RunningChatInfo {
    pub chat_id: u64,
    pub agent: String,
    pub project_key: String,
    pub cwd: String,
    pub session_id: Option<String>,
    pub title: String,
    pub messages: Vec<crate::types::Msg>,
    pub turn_state: String,
    pub turn_started_at_ms: Option<u64>,
    pub permission_mode: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub process_model: String,
}

pub fn list_running_chats() -> Vec<RunningChatInfo> {
    let guard = match map().lock() {
        Ok(g) => g,
        Err(_) => return vec![],
    };
    guard
        .iter()
        .map(|(id, (_handle, meta))| RunningChatInfo {
            chat_id: *id,
            agent: meta.agent.clone(),
            project_key: meta.project_key.clone(),
            cwd: meta.cwd.clone(),
            session_id: meta.session_id.lock().ok().and_then(|g| g.clone()),
            title: meta.title.lock().map(|g| g.clone()).unwrap_or_default(),
            messages: meta.messages.lock().map(|g| g.clone()).unwrap_or_default(),
            turn_state: if meta
                .turn_started_at_ms
                .lock()
                .ok()
                .and_then(|g| *g)
                .is_some()
            {
                "running".to_string()
            } else {
                "idle".to_string()
            },
            turn_started_at_ms: meta.turn_started_at_ms.lock().ok().and_then(|g| *g),
            permission_mode: meta.permission_mode.clone(),
            model: meta.model.clone(),
            effort: meta.effort.clone(),
            process_model: meta.process_model.clone(),
        })
        .collect()
}

pub fn set_title(id: u64, title: String) {
    if let Ok(m) = map().lock() {
        if let Some((_handle, meta)) = m.get(&id) {
            if let Ok(mut t) = meta.title.lock() {
                *t = title;
            }
        }
    }
}
