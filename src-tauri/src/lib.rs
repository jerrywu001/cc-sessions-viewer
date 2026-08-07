// AI 会话管理器 —— 后端入口。
//
// 这个文件只做两件事：
//   1. 注册 Tauri 命令，把请求路由到对应模块（`agents` / `trash`）。
//   2. macOS 启动期 setup（unifiedCompact 标题栏）。
//
// 所有 agent 相关的解析、读写、重命名逻辑都在 `agents/*.rs` 里；
// 回收站逻辑在 `trash.rs`；跨模块共用的小工具在 `util.rs`；
// 跟前端共享的序列化类型在 `types.rs`。
// 接入新 agent 的步骤详见 `agents/mod.rs` 顶部注释。

// agents / stats are `pub` so the `examples/test_dedup.rs` binary (compiled as
// an external consumer of the lib crate) can call into the dedup pipeline
// directly. Everything else stays crate-private.
mod agent_chat;
mod agent_command;
pub mod agents;
mod background_media;
mod bookmarks;
mod claude_config;
mod cli_env;
mod desktop_pet_assets;
mod git;
#[cfg(target_os = "macos")]
mod menu;
mod panic_log;
mod pty;
pub mod stats;
mod trash;
#[cfg(target_os = "macos")]
mod tray;
#[cfg(target_os = "windows")]
mod tray_windows;
mod turn;
mod types;
mod usage_api;
mod util;
mod watch;
mod worktrees;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agent_command::AgentCommand;
use crate::types::{
    AgentStats, ClaudeRuntimeInfo, CodexRuntimeInfo, Msg, ProjectInfo, SearchHit, SessionPage,
    TrashItem, TrayStats, UsageSummary,
};
#[allow(unused_imports)]
use tauri::{Emitter, Manager};

/// 全局搜索的取消代际 —— 每次新搜索把自己的 `request_id` 写进来，正在跑的搜索循环
/// 不停 check；一旦发现 gen ≠ 自己的 id 就主动 bail。`cancel_search()` 直接 bump 它。
static SEARCH_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// ============================ Tauri 命令：分派层 ============================

#[tauri::command]
fn list_projects(
    agent: String,
    include_codex_internal: bool,
    include_codex_archived: bool,
) -> Result<Vec<ProjectInfo>, String> {
    let mut out =
        agents::source(&agent)?.list_projects(include_codex_internal, include_codex_archived)?;
    let bm = bookmarks::load(&agent);
    for bp in bm {
        if out.iter().any(|p| p.display_path == bp) {
            continue;
        }
        let bp_path = Path::new(&bp);
        let exists = bp_path.is_dir();
        let (count, last) = if exists {
            bookmarks::count_sessions(bp_path)
        } else {
            (0, 0)
        };
        out.push(ProjectInfo {
            dir_name: format!("bookmark:{bp}"),
            display_path: bp,
            session_count: count,
            last_modified: last,
            exists,
            bookmarked: true,
            parent_dir_name: None,
            worktree_name: None,
        });
    }
    inject_worktrees(&agent, &mut out);
    Ok(out)
}

/// 支持 worktree 展示/创建的 agent：只有按 `cwd` 归属会话的 Claude / Codex。opencode / agy
/// 按 git 仓库归属会话 —— worktree 里起的会话会被 CLI 塞回主仓库，展示 worktree 反而误导，
/// 故对它们整体隐藏 worktree（显示 + 创建入口，前端也据同一名单收起）。
fn agent_supports_worktrees(agent: &str) -> bool {
    matches!(agent, "claude" | "codex")
}

/// 把磁盘上 `<项目根>/.claude/worktrees/*` 里的 worktree 注入项目列表 —— agent 无关，
/// 四种 agent 侧栏都会显示、且都归组在其仓库名下。分几种情况：
///   * 该 worktree 已有会话 → 已作为普通项目在 `out` 里（display_path 命中）。若尚未带
///     父子标记（非 Claude agent），就地补上 parent_dir_name / worktree_name 让其归组。
///   * 尚无会话 → 注入一个 `worktree:<path>` 合成条目（session_count=0），一样能选中/删除。
///   * 父项目不在当前 agent 列表（该 agent 没在这仓库跑过，或记录的是旧路径）→ 合成一个
///     `worktree-root:<path>` 父项目条目，让 worktree 照样归组在正确的仓库名下，而非平铺成孤儿。
fn inject_worktrees(agent: &str, out: &mut Vec<ProjectInfo>) {
    if !agent_supports_worktrees(agent) {
        return;
    }
    // 候选父根 = 当前 agent 列表里存在、且自身不是 worktree 的项目路径
    //           ∪ 曾建过 worktree 的父根（持久化，agent 无关 → 四端都能看到）。
    let mut roots: Vec<String> = out
        .iter()
        .filter(|p| p.exists && p.worktree_name.is_none())
        .map(|p| p.display_path.clone())
        .collect();
    roots.extend(worktrees::load_roots());
    let found = worktrees::scan(&roots);
    if found.is_empty() {
        return;
    }
    // display_path → dir_name，用于把 worktree 关联到父项目（归一化路径后比较）。
    let path_to_dir: std::collections::HashMap<String, String> = out
        .iter()
        .map(|p| (worktrees::normalize(&p.display_path), p.dir_name.clone()))
        .collect();

    // 为每个用到的父根确定「可归属的父项目 dir_name」：命中现有项目就复用；否则合成一个
    // 父项目条目（下面统一 push），让 worktree 有稳定的父节点归组。
    let mut parent_dir_for: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut synth_parents: Vec<ProjectInfo> = Vec::new();
    for wt in &found {
        if parent_dir_for.contains_key(&wt.parent_path) {
            continue;
        }
        if let Some(dir) = path_to_dir.get(&wt.parent_path) {
            parent_dir_for.insert(wt.parent_path.clone(), dir.clone());
        } else {
            let dir = format!("worktree-root:{}", wt.parent_path);
            synth_parents.push(ProjectInfo {
                dir_name: dir.clone(),
                display_path: wt.parent_path.clone(),
                session_count: 0,
                last_modified: util::mtime_millis(Path::new(&wt.parent_path)),
                exists: Path::new(&wt.parent_path).is_dir(),
                bookmarked: false,
                parent_dir_name: None,
                worktree_name: None,
            });
            parent_dir_for.insert(wt.parent_path.clone(), dir);
        }
    }
    out.extend(synth_parents);

    for wt in found {
        let parent_dir = parent_dir_for.get(&wt.parent_path).cloned();
        // 已有同路径项目（该 worktree 跑过会话）→ 就地补标记，不新建条目。
        if let Some(existing) = out
            .iter_mut()
            .find(|p| worktrees::normalize(&p.display_path) == wt.path)
        {
            if existing.worktree_name.is_none() {
                existing.worktree_name = Some(wt.name.clone());
            }
            if existing.parent_dir_name.is_none() {
                existing.parent_dir_name = parent_dir;
            }
            continue;
        }
        // 零会话 worktree → 合成条目。last_modified 取目录 mtime，排序时不至于总排最前。
        let last = util::mtime_millis(Path::new(&wt.path));
        out.push(ProjectInfo {
            dir_name: format!("worktree:{}", wt.path),
            display_path: wt.path.clone(),
            session_count: 0,
            last_modified: last,
            exists: true,
            bookmarked: false,
            parent_dir_name: parent_dir,
            worktree_name: Some(wt.name),
        });
    }
}

#[tauri::command]
fn list_sessions(
    agent: String,
    project_key: String,
    offset: usize,
    limit: usize,
    include_codex_internal: bool,
    include_codex_archived: bool,
) -> Result<SessionPage, String> {
    if let Some(bm_path) = project_key.strip_prefix("bookmark:") {
        return bookmarks::list_sessions_in_dir(bm_path, offset, limit);
    }
    // 零会话 worktree 及合成父项目的 key —— 自身无 transcript，返回空页。worktree 跑过会话后
    // 会以普通项目 key 出现（display_path 命中），不再走这里。
    if project_key.starts_with("worktree:") || project_key.starts_with("worktree-root:") {
        return Ok(SessionPage {
            total: 0,
            sessions: vec![],
        });
    }
    agents::source(&agent)?.list_sessions(
        &project_key,
        offset,
        limit,
        include_codex_internal,
        include_codex_archived,
    )
}

#[tauri::command]
fn read_session(agent: String, path: String) -> Result<Vec<Msg>, String> {
    agents::source(&agent)?.read_session(&path)
}

#[tauri::command]
fn codex_archive_session(session_id: String) -> Result<(), String> {
    if !session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("Invalid session id".to_string());
    }
    let output = std::process::Command::new("codex")
        .arg("archive")
        .arg(&session_id)
        .output()
        .map_err(|e| format!("failed to run codex archive: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("codex archive failed: {stderr}"))
    }
}

/// 实时 tail：开始监听 path 文件的写入事件。
/// 同一时刻只允许一个 watch；再次调用会替换上一个 watcher。
/// 文件不存在返回 Err，前端可以静默降级（仅一次性读取）。
#[tauri::command]
fn watch_session(app: tauri::AppHandle, agent: String, path: String) -> Result<(), String> {
    watch::watch_session(app, agent, path)
}

/// 停止当前 tail；空操作可重入。前端 unmount / 切会话时调用。
#[tauri::command]
fn unwatch_session() -> Result<(), String> {
    watch::unwatch_session()
}

#[tauri::command]
fn check_watched_session(app: tauri::AppHandle) -> Result<(), String> {
    watch::check_watched_session(app)
}

#[tauri::command]
fn terminal_turn_signal(
    app: tauri::AppHandle,
    agent: String,
    path: String,
    state: String,
) -> Result<(), String> {
    turn::emit_turn_signal(
        &app,
        turn::TerminalTurnPayload {
            agent,
            path,
            state,
            source: "hook".to_string(),
        },
    )
}

#[tauri::command]
fn install_turn_hooks() -> Result<turn::TurnHookInstallResult, String> {
    turn::install_turn_hooks()
}

#[tauri::command]
fn turn_hook_status() -> Result<turn::TurnHookStatus, String> {
    turn::turn_hook_status()
}

#[tauri::command]
fn desktop_pet_tasks() -> Result<Vec<turn::DesktopTask>, String> {
    turn::desktop_task_snapshot()
}

#[derive(Clone, serde::Serialize)]
struct DesktopPetSessionTarget {
    agent: String,
    path: String,
}

#[tauri::command]
fn acknowledge_desktop_pet_task(
    app: tauri::AppHandle,
    agent: String,
    path: String,
) -> Result<(), String> {
    if turn::acknowledge_desktop_task_by_path(&agent, &path)? {
        app.emit(
            "desktop-pet://activity-acknowledged",
            DesktopPetSessionTarget { agent, path },
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn resolve_desktop_pet_session(
    agent: String,
    path: String,
) -> Result<Option<turn::DesktopPetResolvedSession>, String> {
    turn::resolve_desktop_pet_session(&agent, &path)
}

fn position_desktop_pet(window: &tauri::WebviewWindow) -> Result<(), String> {
    let Some(monitor) = window
        .primary_monitor()
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let window_size = window.outer_size().map_err(|error| error.to_string())?;
    let margin = 24;
    let x = monitor_position.x + monitor_size.width as i32 - window_size.width as i32 - margin;
    let y = monitor_position.y + monitor_size.height as i32 - window_size.height as i32 - margin;
    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            x.max(monitor_position.x),
            y.max(monitor_position.y),
        )))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn set_desktop_pet_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if !enabled {
        if let Some(window) = app.get_webview_window("desktop-pet") {
            window.close().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    if let Some(window) = app.get_webview_window("desktop-pet") {
        window.show().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "desktop-pet",
        tauri::WebviewUrl::App("index.html?desktop-pet=1".into()),
    )
    .title("Codex Pet")
    .inner_size(258.0, 138.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;
    position_desktop_pet(&window)?;
    window.show().map_err(|error| error.to_string())
}

#[tauri::command]
fn focus_desktop_pet_main(app: tauri::AppHandle) -> Result<(), String> {
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is unavailable".to_string())?;
    main.show().map_err(|error| error.to_string())?;
    main.unminimize().map_err(|error| error.to_string())?;
    main.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn open_desktop_pet_session(
    app: tauri::AppHandle,
    agent: String,
    path: String,
) -> Result<(), String> {
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is unavailable".to_string())?;
    main.show().map_err(|error| error.to_string())?;
    main.unminimize().map_err(|error| error.to_string())?;
    main.set_focus().map_err(|error| error.to_string())?;
    main.emit(
        "desktop-pet://open-session",
        DesktopPetSessionTarget { agent, path },
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn claude_runtime_info() -> Result<ClaudeRuntimeInfo, String> {
    claude_config::runtime_info()
}

/// Codex 运行时信息：检测是否通过第三方 API key / 自定义端点使用（config.toml 里
/// `model_provider` 为 "custom" 或存在 `[model_providers.*]` 配置）。前端据此隐藏
/// 仅官方订阅可用的模型（如 GPT-5.3-Codex-Spark）。
#[tauri::command]
fn codex_runtime_info() -> CodexRuntimeInfo {
    let config_path = util::home().join(".codex").join("config.toml");
    let content = fs::read_to_string(&config_path).unwrap_or_default();
    let uses_api_key = content.lines().any(|l| {
        let l = l.trim();
        l.starts_with("model_provider")
            && l.contains('=')
            && !l.starts_with('#')
            && !l.starts_with('[')
    });
    CodexRuntimeInfo {
        uses_api_key,
        model: top_level_toml_string(&content, "model"),
        effort: top_level_toml_string(&content, "model_reasoning_effort"),
    }
}

fn top_level_toml_string(content: &str, key: &str) -> Option<String> {
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            break;
        }
        let Some((k, value)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        return parse_toml_string_value(value.trim());
    }
    None
}

fn parse_toml_string_value(value: &str) -> Option<String> {
    let value = value.trim_start();
    if !value.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    let mut out = String::new();
    for ch in value[1..].chars() {
        if escaped {
            let decoded = match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            };
            out.push(decoded);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

#[cfg(test)]
mod codex_runtime_tests {
    use super::top_level_toml_string;

    #[test]
    fn reads_top_level_codex_model() {
        let content = r#"
model_provider = "custom"
model = "gpt-5.6-sol"
model_reasoning_effort = "high"

[model_providers.custom]
model = "provider-level"
"#;
        assert_eq!(
            top_level_toml_string(content, "model").as_deref(),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            top_level_toml_string(content, "model_reasoning_effort").as_deref(),
            Some("high")
        );
    }

    #[test]
    fn ignores_section_values() {
        let content = r#"
[model_providers.custom]
model = "provider-level"
"#;
        assert_eq!(top_level_toml_string(content, "model"), None);
    }
}

/// 单个会话的 token 用量汇总（按 path + mtime 缓存）。
/// 前端 ChatTopbar / SessionsView 卡片懒加载这条。
#[tauri::command]
fn session_usage(agent: String, path: String) -> Result<UsageSummary, String> {
    let src = agents::source(&agent)?;
    agents::session_usage(&*src, &path)
}

/// 「当前上下文」用量 —— 取会话最后一条 usage（≈末尾上下文规模），而非全程累加。
/// 续聊（resume）时前端拿它给上下文进度角标做种子，否则刚切过去会显示 0% 与 TUI 不符。
#[tauri::command]
fn session_context_usage(agent: String, path: String) -> Result<UsageSummary, String> {
    let src = agents::source(&agent)?;
    src.context_usage(&path)
}

#[tauri::command]
fn session_last_prompt(agent: String, path: String) -> Result<Option<String>, String> {
    let src = agents::source(&agent)?;
    src.last_prompt(&path)
}

/// 当前 agent 的统计概览：顶层标量 + 项目排行（按 token 降序）+ 日活时间轴。
/// **保留作兼容入口** —— 旧版同步路径仍然可用，但内容比 start_agent_stats 简化（没有
/// cost / by_model / by_tool 等）。前端默认走流式接口，这里只作兜底。
#[tauri::command]
fn agent_stats(agent: String) -> Result<AgentStats, String> {
    let src = agents::source(&agent)?;
    agents::agent_stats(&*src, &agent)
}

/// 流式启动一次统计扫描。函数立刻返回；后台 worker 通过 `stats://progress` /
/// `stats://done` / `stats://error` 三个事件把结果推回前端。新请求会让旧请求让位
/// （`STATS_GEN` 代际计数器）。前端用 `requestId` 比对，丢掉旧数据。
///
/// `scope`：`all` / `claude` / `codex` / `session:<agent>:<absolute path>`。
/// `range`：`today` / `days7` / `days30` / `month` / `months3` / `months6` /
/// `custom:YYYY-MM-DD:YYYY-MM-DD`（session-scope 下忽略）。
#[tauri::command]
fn start_agent_stats(app: tauri::AppHandle, scope: String, range: String, request_id: u64) {
    stats::stream::start(app, scope, range, request_id);
}

/// 立刻取消任何正在跑的统计 worker。本质上是把全局代际 +1，跑中的 worker 自己 bail。
#[tauri::command]
fn cancel_stats() {
    stats::stream::cancel();
}

/// 全局搜索：跨当前 agent 的所有项目 / 会话查关键词。
/// 命中范围在 `agents::search` 里：标题 / id / 项目路径 / 文本（仅 text + thinking 块）；
/// 工具调用 / 工具结果 / 文件改动默认不参与匹配。
/// 空字符串返回空数组（避免一次性把所有会话当结果返回）。
///
/// **可取消**：每次调用都会把 `request_id` 写进全局 SEARCH_GEN；之后任何 `cancel_search()`
/// 或更大 id 的 `search_sessions` 都会让旧的搜索循环立刻 bail（返回空数组）。前端的
/// reqSeq 守卫负责丢掉过期结果，所以即使后端返回了一堆结果也不会污染 UI。
#[tauri::command]
async fn search_sessions(
    agent: String,
    query: String,
    request_id: u64,
    project_key: Option<String>,
) -> Result<Vec<SearchHit>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        SEARCH_GEN.store(request_id, std::sync::atomic::Ordering::SeqCst);
        let src = agents::source(&agent)?;
        let cancel = agents::Cancel {
            request_id,
            gen: &SEARCH_GEN,
        };
        agents::search(&*src, &query, project_key.as_deref(), cancel)
    })
    .await
    .map_err(|e| format!("search task panicked: {e}"))?
}

/// 显式取消正在跑的全局搜索 —— 前端每次新输入立即调一次，让 CPU 让位给打字。
/// 仅仅 bump 一下 SEARCH_GEN —— 在跑的 search 循环下次 check 时就会 bail。
#[tauri::command]
fn cancel_search() {
    SEARCH_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// 重命名会话：与 Claude Code `/rename` / Codex 内部重命名一致，
/// 在原 JSONL 末尾追加一条官方 schema 的元数据行（append-only），
/// 后续扫描时取最后一条 `custom-title` / `thread_name_updated` 作为标题。
/// 各 agent 还可能写额外的旁路文件（codex 会同步更新 session_index.jsonl / state_<N>.sqlite）。
#[tauri::command]
fn rename_session(agent: String, path: String, name: String) -> Result<(), String> {
    let fp = PathBuf::from(&path);
    let src = agents::source(&agent)?;
    // 路径合法性由 agent 自查：文件型 = 存在且 .jsonl；opencode = opencode:// 虚拟路径。
    src.validate_session_path(&fp)?;
    src.rename_session(&fp, &name)
}

/// `/fork`：把既有会话克隆成一个全新、独立的磁盘 transcript（新 session id），打上 `title`，
/// 返回新 session id。`source_id` 会被插进文件名 → 用与 resume 同款 `[A-Za-z0-9-]+` 白名单
/// 拦掉路径穿越。仅 Claude 实现派生语义，其它 agent 经 trait 默认实现报错。
#[tauri::command]
fn fork_session(
    agent: String,
    project_key: String,
    source_id: String,
    title: String,
) -> Result<String, String> {
    if source_id.is_empty()
        || !source_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid session ID".to_string());
    }
    agents::source(&agent)?.fork_session(&project_key, &source_id, &title)
}

/// 取消后编辑原提问：只复制目标用户提问之前的 Claude transcript。
#[tauri::command]
fn fork_session_before_user_turn(
    agent: String,
    project_key: String,
    source_id: String,
    title: String,
    keep_user_turns: usize,
) -> Result<String, String> {
    if source_id.is_empty()
        || !source_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid session ID".to_string());
    }
    agents::source(&agent)?.fork_session_before_user_turn(
        &project_key,
        &source_id,
        &title,
        keep_user_turns,
    )
}

#[tauri::command]
fn soft_delete_session(agent: String, path: String, project_label: String) -> Result<(), String> {
    trash::soft_delete(&agent, &path, &project_label)
}

/// 永久删除一个会话文件（直接 rm，不进回收站、不可恢复）。仅供 worktree「全部删除」调用。
/// 路径经 agent 的 `validate_session_path` 校验（存在 + 是 JSONL），防止误删任意文件。
/// 删完后若其所在目录（如 `~/.claude/projects/<worktree 编码目录>` —— worktree 的全局工作
/// 目录）已空，一并移除；只删空目录，绝不误伤仍有会话的目录。
#[tauri::command]
fn hard_delete_session(agent: String, path: String) -> Result<(), String> {
    let src = agents::source(&agent)?;
    let fp = PathBuf::from(&path);
    src.validate_session_path(&fp)?;
    fs::remove_file(&fp).map_err(|e| format!("Failed to delete session: {e}"))?;
    if let Some(parent) = fp.parent() {
        let empty = fs::read_dir(parent)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if empty {
            let _ = fs::remove_dir(parent);
        }
    }
    Ok(())
}

/// btw 侧聊关闭后清理 `--fork-session` 产生的会话文件。只认 Claude agent，
/// 路径固定为 `~/.claude/projects/<project_key>/<session_id>.jsonl`。session_id
/// 做严格白名单校验，防路径穿越。
#[tauri::command]
fn purge_btw_session(project_key: String, session_id: String) -> Result<(), String> {
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid session ID".to_string());
    }
    let fp = crate::util::home()
        .join(".claude")
        .join("projects")
        .join(&project_key)
        .join(format!("{session_id}.jsonl"));
    if fp.is_file() {
        fs::remove_file(&fp).map_err(|e| format!("Failed to purge btw session: {e}"))?;
    }
    // 同伴子目录 <sessionId>/subagents/ 也一并清除
    let companion = fp.with_extension("");
    if companion.is_dir() {
        let _ = fs::remove_dir_all(&companion);
    }
    Ok(())
}

/// 在 `project_path` 下新建一个 git worktree（同名新分支），落到
/// `<project_path>/.claude/worktrees/<name>`。返回新 worktree 的绝对路径。
#[tauri::command]
fn create_worktree(project_path: String, name: String) -> Result<String, String> {
    worktrees::create(&project_path, &name)
}

/// 全部删除 `path` 处的 worktree（工作树 + 分支，不可撤销）。会话记录由前端在调用前
/// 软删到回收站。
#[tauri::command]
fn remove_worktree(path: String) -> Result<(), String> {
    worktrees::remove(&path)
}

/// 强制删除各 agent 在 `worktree_path` 下残留的项目目录。worktree 是跨 agent 共享的，
/// 删除时需要连带清理所有 agent 的元数据目录，否则会残留孤儿。
#[tauri::command]
fn cleanup_worktree_project_dirs(worktree_path: String) -> Result<(), String> {
    worktrees::cleanup_project_dirs(&worktree_path)
}

#[tauri::command]
fn list_trash() -> Result<Vec<TrashItem>, String> {
    trash::list()
}

#[tauri::command]
fn restore_session(trash_file: String) -> Result<(), String> {
    trash::restore(&trash_file)
}

#[tauri::command]
fn permanent_delete_trash(trash_file: String) -> Result<(), String> {
    trash::permanent_delete(&trash_file)
}

#[tauri::command]
fn empty_trash() -> Result<(), String> {
    trash::empty()
}

/// 内嵌 TUI：在窗口内部的 xterm.js 里跑 `<shell> -l -c "cd <cwd> && <resume CLI>"`。
/// 返回新 PTY 的内部 id；前端拿 id 调 `pty_write` / `pty_resize` / `pty_kill`。
/// 与 `resume_session`（开 Terminal.app）并存 —— 调用方各自决定走哪一条。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn pty_spawn(
    app: tauri::AppHandle,
    agent: String,
    session_id: String,
    cwd: String,
    path: String,
    cols: u16,
    rows: u16,
    extra_args: String,
    color_scheme: Option<String>,
    use_reclaude: Option<bool>,
) -> Result<u64, String> {
    if !Path::new(&cwd).is_dir() {
        return Err("Project directory no longer exists".to_string());
    }
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid session ID".to_string());
    }
    let command = agents::source(&agent)?
        .resume_command(&session_id, &path)
        .with_extra_args(&extra_args);
    pty::spawn(
        app,
        cwd,
        command,
        cols,
        rows,
        color_scheme.as_deref(),
        use_reclaude.unwrap_or(false),
    )
}

/// 启动一个 “new session” PTY（不带 --resume）。session_id 不需要 —— 由 CLI 自己生成新 id。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn pty_spawn_new(
    app: tauri::AppHandle,
    agent: String,
    cwd: String,
    cols: u16,
    rows: u16,
    extra_args: String,
    color_scheme: Option<String>,
    use_reclaude: Option<bool>,
) -> Result<u64, String> {
    if !Path::new(&cwd).is_dir() {
        return Err("Project directory no longer exists".to_string());
    }
    let command = agents::source(&agent)?
        .new_session_command()
        .with_extra_args(&extra_args);
    pty::spawn(
        app,
        cwd,
        command,
        cols,
        rows,
        color_scheme.as_deref(),
        use_reclaude.unwrap_or(false),
    )
}

/// 启动一个纯 shell PTY（不跑任何 agent CLI），用于在项目目录里执行任意命令。
#[tauri::command]
fn pty_spawn_shell(
    app: tauri::AppHandle,
    cwd: String,
    cols: u16,
    rows: u16,
    color_scheme: Option<String>,
) -> Result<u64, String> {
    if !Path::new(&cwd).is_dir() {
        return Err("Project directory no longer exists".to_string());
    }
    pty::spawn_shell(app, cwd, cols, rows, color_scheme.as_deref())
}

#[tauri::command]
fn pty_write(id: u64, data: String) -> Result<(), String> {
    pty::write(id, &data)
}

#[tauri::command]
fn pty_resize(id: u64, cols: u16, rows: u16) -> Result<(), String> {
    pty::resize(id, cols, rows)
}

#[tauri::command]
fn pty_kill(id: u64) -> Result<(), String> {
    pty::kill(id)
}

// ---------- 程序化聊天（GUI chat）：管道子进程跑 stream-json ----------

/// model / effort flag 值的轻校验：非空、≤64、仅 `[A-Za-z0-9._:-]`。值最终经
/// `AgentCommand` 的 posix_quote 安全转义（不会注入），这里只是额外挡掉明显垃圾，
/// 并与前端候选列表对齐。低 / 高 / xhigh / max / minimal、gpt-5.1-codex-max 等均通过。
fn valid_flag_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// 权限模式允许列表（会进 shell 命令；虽已 posix_quote，仍只放行已知值）。
/// Claude：对齐 `claude --permission-mode` 的 choices（含 auto / dontAsk）。
/// Codex：独立四档 ask / approve / fullAccess / custom。
fn valid_permission_mode(mode: &str) -> bool {
    matches!(
        mode,
        "default"
            | "acceptEdits"
            | "plan"
            | "auto"
            | "dontAsk"
            | "bypassPermissions"
            | "ask"
            | "approve"
            | "fullAccess"
            | "custom"
    )
}

fn default_permission_mode_for_agent(agent: &str) -> &'static str {
    if agent == "codex" {
        "fullAccess"
    } else {
        "bypassPermissions"
    }
}

/// 启动一个 GUI chat 子进程，返回 chat id + 进程模型。`session_id` 给出时续聊既有
/// 会话；`permission_mode` / `model` / `effort` 走校验后透传给 CLI（权限默认按 agent
/// 取最大档，model/effort 为空走 CLI 自身默认）。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn agent_chat_start(
    app: tauri::AppHandle,
    agent: String,
    project_key: String,
    cwd: String,
    session_id: Option<String>,
    permission_mode: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    fork: Option<bool>,
    use_reclaude: Option<bool>,
    preload_messages: Option<Vec<crate::types::Msg>>,
    title: Option<String>,
    ephemeral: Option<bool>,
) -> Result<crate::types::ChatStartInfo, String> {
    if !Path::new(&cwd).is_dir() {
        return Err("Project directory no longer exists".to_string());
    }
    // 续聊时校验 session id（会被拼进 --resume）。新开会话 session_id 为空。
    if let Some(id) = session_id.as_deref() {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("Invalid session ID".to_string());
        }
    }
    let mode =
        permission_mode.unwrap_or_else(|| default_permission_mode_for_agent(&agent).to_string());
    if !valid_permission_mode(&mode) {
        return Err("Invalid permission mode".to_string());
    }
    if let Some(m) = model.as_deref() {
        if !valid_flag_token(m) {
            return Err("Invalid model".to_string());
        }
    }
    if let Some(e) = effort.as_deref() {
        if !valid_flag_token(e) {
            return Err("Invalid effort".to_string());
        }
    }
    // 进程模型在 start 移走 agent 之前算（前端据此决定切设置走 restart 还是下轮 flag）。
    let process_model = agents::source(&agent)?
        .chat_process_model()
        .as_str()
        .to_string();
    // start() 会**同步阻塞**：Codex 路径要 ensure_codex_cli_available（起 PowerShell 校验 PATH）
    // + app-server 握手（init / thread.start，实测 ~1-2s）。若留在同步命令里，Tauri 会在主线程上
    // 跑它 → 期间整个 webview 假死、鼠标转圈。挪到 blocking 线程池，主线程（UI）保持响应。
    let chat_id = tauri::async_runtime::spawn_blocking(move || {
        agent_chat::start(
            app,
            agent,
            project_key,
            cwd,
            session_id,
            mode,
            model,
            effort,
            fork.unwrap_or(false),
            ephemeral.unwrap_or(false),
            use_reclaude.unwrap_or(false),
            preload_messages,
            title,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(crate::types::ChatStartInfo {
        chat_id,
        process_model,
    })
}

#[tauri::command]
fn reclaude_info() -> crate::types::ReclaudeInfo {
    agent_chat::reclaude_info()
}

#[tauri::command]
fn agent_chat_list_running() -> Vec<agent_chat::RunningChatInfo> {
    agent_chat::list_running_chats()
}

/// 向某个 chat 子进程发送一条用户消息（含可选图片附件 + 本轮的 model/effort/权限）。
/// one-shot agent（Codex）据此每轮切换；长驻 agent（Claude）这三者在 start 已定型，
/// 后端忽略（切换走 restart-with-resume）。
#[tauri::command]
fn agent_chat_send(
    id: u64,
    text: String,
    images: Option<Vec<crate::types::ChatImageInput>>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    text_elements: Option<Vec<serde_json::Value>>,
) -> Result<(), String> {
    if let Some(m) = model.as_deref() {
        if !valid_flag_token(m) {
            return Err("Invalid model".to_string());
        }
    }
    if let Some(e) = effort.as_deref() {
        if !valid_flag_token(e) {
            return Err("Invalid effort".to_string());
        }
    }
    let mode = permission_mode.unwrap_or_else(|| "acceptEdits".to_string());
    if !valid_permission_mode(&mode) {
        return Err("Invalid permission mode".to_string());
    }
    agent_chat::send(
        id,
        &text,
        &images.unwrap_or_default(),
        model.as_deref(),
        effort.as_deref(),
        &mode,
        &text_elements.unwrap_or_default(),
    )
}

/// 将一条已排队消息追加给当前运行的 Codex app-server turn，不创建新 turn、也不中断当前执行。
#[tauri::command]
fn agent_chat_steer(
    id: u64,
    text: String,
    text_elements: Option<Vec<serde_json::Value>>,
) -> Result<(), String> {
    agent_chat::steer(id, &text, &text_elements.unwrap_or_default())
}

#[tauri::command]
async fn agent_chat_fork_before_last_turn(id: u64) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || agent_chat::fork_before_last_turn(id))
        .await
        .map_err(|e| e.to_string())?
}

/// 结束一个 chat 子进程（kill + 回收）。幂等。
#[tauri::command]
fn agent_chat_stop(id: u64) -> Result<(), String> {
    agent_chat::stop(id)
}

#[tauri::command]
fn agent_chat_set_title(id: u64, title: String) {
    agent_chat::set_title(id, title);
}

#[tauri::command]
fn agent_chat_interrupt(id: u64) -> Result<(), String> {
    agent_chat::interrupt(id)
}

/// 回写一次交互式工具权限决定（应答 `agent-chat://permission`）。`request_id` 来自该次
/// 请求；`decision` 是前端构造好的 `{behavior:"allow"|"deny",...}`（结构由 CLI 控制协议
/// 决定，后端只做透传校验：必须是 JSON 对象且 behavior 合法）。
#[tauri::command]
fn agent_chat_respond_permission(
    id: u64,
    request_id: String,
    decision: serde_json::Value,
) -> Result<(), String> {
    if request_id.is_empty() {
        return Err("Invalid request id".to_string());
    }
    match decision.get("behavior").and_then(|b| b.as_str()) {
        Some("allow") | Some("deny") => {}
        _ => return Err("Invalid permission decision".to_string()),
    }
    agent_chat::respond_permission(id, &request_id, decision)
}

/// 回写一次结构化提问（AskUserQuestion）的答案（应答 `agent-chat://question`）。`decision`
/// 是前端构造好的 `{behavior:"allow",updatedInput:{questions,answers,response?}}`（作答）或
/// `{behavior:"deny",...}`（取消），后端只做透传校验：必须带合法 behavior。
#[tauri::command]
fn agent_chat_respond_question(
    id: u64,
    request_id: String,
    decision: serde_json::Value,
) -> Result<(), String> {
    if request_id.is_empty() {
        return Err("Invalid request id".to_string());
    }
    match decision.get("behavior").and_then(|b| b.as_str()) {
        Some("allow") | Some("deny") => {}
        _ => return Err("Invalid question decision".to_string()),
    }
    agent_chat::respond_question(id, &request_id, decision)
}

/// GUI chat 输入框 `/` 浮层的动态指令列表（扫磁盘自定义命令 / user-invocable skills）。
#[tauri::command]
fn agent_chat_slash_commands(
    agent: String,
    cwd: String,
) -> Result<Vec<crate::types::SlashCommand>, String> {
    Ok(agents::source(&agent)?.chat_slash_commands(&cwd))
}

/// 在终端中用对应 CLI 恢复（resume）一个会话。
#[tauri::command]
fn resume_session(
    agent: String,
    session_id: String,
    cwd: String,
    path: String,
    extra_args: String,
    terminal_app: String,
) -> Result<(), String> {
    if !Path::new(&cwd).is_dir() {
        return Err("Project directory no longer exists".to_string());
    }
    // id 校验：Claude/Codex 为 UUID
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid session ID".to_string());
    }
    let command = agents::source(&agent)?
        .resume_command(&session_id, &path)
        .with_extra_args(&extra_args);
    spawn_terminal(&command, &cwd, &terminal_app)
}

/// 在终端里为某个项目目录开一个全新会话（不带 --resume）。
#[tauri::command]
fn new_session(
    agent: String,
    cwd: String,
    extra_args: String,
    terminal_app: String,
) -> Result<(), String> {
    if !Path::new(&cwd).is_dir() {
        return Err("Project directory no longer exists".to_string());
    }
    let command = agents::source(&agent)?
        .new_session_command()
        .with_extra_args(&extra_args);
    spawn_terminal(&command, &cwd, &terminal_app)
}

/// 通过用户登录 shell 解析命令的完整路径。
/// 打包后的 .app 不继承用户 PATH，直接 `Command::new("cmux")` 会 ENOENT。
/// 对于 macOS GUI app（如 cmux），登录 shell 也可能找不到 —— 用已知安装路径兜底。
#[cfg(unix)]
fn resolve_bin(name: &str) -> Result<PathBuf, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = std::process::Command::new(&shell)
        .args(["-l", "-c", &format!("which {name}")])
        .output()
        .map_err(|e| format!("Failed to resolve {name} via shell: {e}"))?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && !path.is_empty() {
        return Ok(PathBuf::from(path));
    }
    // macOS GUI app 内置的 CLI 二进制不在 PATH 里 —— 逐个检查已知路径。
    let known_paths: &[&str] = match name {
        "cmux" => &["/Applications/cmux.app/Contents/Resources/bin/cmux"],
        _ => &[],
    };
    for p in known_paths {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    Err(format!(
        "{name} not found — make sure it is installed and in your PATH"
    ))
}

#[cfg(target_os = "macos")]
fn create_terminal_script(tab_name: &str, shell_cmd: &str) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join("cc-sessions-viewer");
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp dir: {e}"))?;
    let path = dir.join(format!("resume-{}.command", std::process::id()));
    let content = format!(
        "#!/bin/zsh\n\
         printf '\\033]0;{tab_name}\\007'\n\
         {shell_cmd}\n"
    );
    fs::write(&path, &content).map_err(|e| format!("Failed to write script: {e}"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("Failed to set permissions: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

fn spawn_terminal(command: &AgentCommand, cwd: &str, _terminal_app: &str) -> Result<(), String> {
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST_SPAWN: Mutex<Option<(String, Instant)>> = Mutex::new(None);
    {
        let mut last = LAST_SPAWN.lock().unwrap();
        if let Some((ref prev_cwd, t)) = *last {
            if prev_cwd == cwd && t.elapsed().as_millis() < 2000 {
                return Ok(());
            }
        }
        *last = Some((cwd.to_string(), Instant::now()));
    }

    #[cfg(target_os = "macos")]
    {
        let cli = command.to_posix_shell();
        let shell_cmd = format!("cd {} && {}", crate::agent_command::posix_quote(cwd), cli);

        match _terminal_app {
            // Ghostty macOS 没有窗口管理 API，无法按 cwd 复用已有窗口，
            // 每次都会开新实例。等 Ghostty 支持 IPC 后再实现窗口复用。
            "ghostty" => {
                std::process::Command::new("open")
                    .args([
                        "-na",
                        "Ghostty.app",
                        "--args",
                        &format!("--working-directory={cwd}"),
                        "-e",
                        "bash",
                        "-lc",
                    ])
                    .arg(&cli)
                    .spawn()
                    .map_err(|e| format!("Failed to launch Ghostty: {e}"))?;
            }
            "cmux" => {
                let cmux_bin = resolve_bin("cmux")?;
                let found_ref = std::process::Command::new(&cmux_bin)
                    .args(["workspace", "list", "--json"])
                    .env("CMUX_QUIET", "1")
                    .output()
                    .ok()
                    .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
                    .and_then(|json| {
                        json["workspaces"]
                            .as_array()?
                            .iter()
                            .find(|w| w["current_directory"].as_str() == Some(cwd))
                            .and_then(|w| w["ref"].as_str().map(String::from))
                    });

                if let Some(ws_ref) = found_ref {
                    // 从结构化参数提取会话 ID（UUID-like token）用于去重。
                    let session_id = command_session_id(command);

                    // 检查 workspace 里是否已有运行这个会话的 surface
                    let existing_surface = session_id.and_then(|sid| {
                        let o = std::process::Command::new(&cmux_bin)
                            .args([
                                "rpc",
                                "surface.list",
                                &format!("{{\"workspace_id\":\"{ws_ref}\"}}"),
                            ])
                            .output()
                            .ok()?;
                        let json: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
                        json["surfaces"].as_array()?.iter().find_map(|s| {
                            let title = s["title"].as_str().unwrap_or("");
                            let checkpoint =
                                s["resume_binding"]["checkpoint_id"].as_str().unwrap_or("");
                            let cmd = s["resume_binding"]["command"].as_str().unwrap_or("");
                            if title.contains(sid) || checkpoint == sid || cmd.contains(sid) {
                                Some((
                                    s["pane_ref"].as_str()?.to_string(),
                                    s["ref"].as_str()?.to_string(),
                                ))
                            } else {
                                None
                            }
                        })
                    });

                    if let Some((_pane_ref, surface_ref)) = existing_surface {
                        let _ = std::process::Command::new(&cmux_bin)
                            .args(["workspace", "select", &ws_ref])
                            .output();
                        let _ = std::process::Command::new(&cmux_bin)
                            .args([
                                "rpc",
                                "surface.focus",
                                &format!("{{\"workspace_id\":\"{ws_ref}\",\"surface_id\":\"{surface_ref}\"}}"),
                            ])
                            .output();
                        let _ = std::process::Command::new(&cmux_bin)
                            .args([
                                "trigger-flash",
                                "--workspace",
                                &ws_ref,
                                "--surface",
                                &surface_ref,
                            ])
                            .spawn();
                    } else {
                        // 新开 split
                        let _ = std::process::Command::new(&cmux_bin)
                            .args(["workspace", "select", &ws_ref])
                            .output();

                        let split_dir = std::process::Command::new(&cmux_bin)
                            .args([
                                "rpc",
                                "pane.list",
                                &format!("{{\"workspace_id\":\"{ws_ref}\"}}"),
                            ])
                            .output()
                            .ok()
                            .and_then(|o| {
                                serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()
                            })
                            .and_then(|json| {
                                let pane = json["panes"]
                                    .as_array()?
                                    .iter()
                                    .find(|p| p["focused"].as_bool() == Some(true))?;
                                let w = pane["pixel_frame"]["width"].as_f64()?;
                                let h = pane["pixel_frame"]["height"].as_f64()?;
                                Some(if w >= h { "right" } else { "down" })
                            })
                            .unwrap_or("down");

                        let _ = std::process::Command::new(&cmux_bin)
                            .args([
                                "new-split",
                                split_dir,
                                "--workspace",
                                &ws_ref,
                                "--focus",
                                "true",
                            ])
                            .output();
                        let _ = std::process::Command::new(&cmux_bin)
                            .args(["send", "--workspace", &ws_ref, cli.as_str()])
                            .output();
                        std::process::Command::new(&cmux_bin)
                            .args(["send-key", "--workspace", &ws_ref, "enter"])
                            .spawn()
                            .map_err(|e| format!("Failed to launch cmux: {e}"))?;
                    }
                } else {
                    let ws_name = Path::new(cwd)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let mut args = vec![
                        "new-workspace",
                        "--cwd",
                        cwd,
                        "--command",
                        cli.as_str(),
                        "--focus",
                        "true",
                    ];
                    if !ws_name.is_empty() {
                        args.push("--name");
                        args.push(&ws_name);
                    }
                    std::process::Command::new(&cmux_bin)
                        .args(&args)
                        .spawn()
                        .map_err(|e| format!("Failed to launch cmux: {e}"))?;
                }
            }
            // iTerm2 / Warp / Terminal.app: 写临时脚本 + open -a，不需要辅助功能权限
            _ => {
                let app_name = match _terminal_app {
                    "iterm2" => "iTerm",
                    "warp" => "Warp",
                    _ => "Terminal",
                };
                let tab_name = Path::new(cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let script_path = create_terminal_script(&tab_name, &shell_cmd)?;
                std::process::Command::new("open")
                    .args(["-a", app_name, &script_path])
                    .spawn()
                    .map_err(|e| format!("Failed to launch {app_name}: {e}"))?;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // 编码后的命令里已含 powershell_refresh_path()：起来的 powershell 会以继承到的
        // $processPath 打头、再并上注册表 User + Machine PATH，无需在 cmd 这层注入 PATH
        // （注入反而会覆盖掉继承来的完整 PATH，留下未展开的注册表字面量）。
        let ps_cmd = crate::agent_command::powershell_set_location_and_run(cwd, command, false);
        let encoded = crate::agent_command::powershell_encoded_command(&ps_cmd);
        let ps_exe = crate::agent_command::windows_powershell_exe();
        // -ExecutionPolicy Bypass（仅本进程）：放行 npm 装的 claude/codex .ps1 垫片，
        // 否则 Win 默认 Restricted 策略会以 UnauthorizedAccess 拒跑 resume 命令。
        // silent_command 只隐藏宿主 cmd 的黑框；`start` 仍会为 powershell 新开可见终端窗口。
        crate::util::silent_command("cmd")
            .args([
                "/c",
                "start",
                "",
                ps_exe,
                "-NoExit",
                "-ExecutionPolicy",
                "Bypass",
                "-EncodedCommand",
                &encoded,
            ])
            .spawn()
            .map_err(|e| format!("Failed to launch terminal: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        let shell_cmd = format!(
            "cd {} && {}",
            crate::agent_command::posix_quote(cwd),
            command.to_posix_shell()
        );
        let terminals = ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"];
        let mut launched = false;
        for term in &terminals {
            let result = if *term == "gnome-terminal" {
                std::process::Command::new(term)
                    .args(["--", "bash", "-c", &shell_cmd])
                    .spawn()
            } else {
                std::process::Command::new(term)
                    .args([
                        "-e",
                        &format!("bash -c '{}'", shell_cmd.replace('\'', "'\\''")),
                    ])
                    .spawn()
            };
            if result.is_ok() {
                launched = true;
                break;
            }
        }
        if !launched {
            return Err("No terminal emulator found".to_string());
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn command_session_id(command: &AgentCommand) -> Option<&str> {
    command.args().iter().find_map(|arg| {
        (arg.len() > 8
            && (arg.contains('-') || arg.contains('_'))
            && arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .then_some(arg.as_str())
    })
}

/// 检测 macOS 上已安装的外部终端应用。返回可用终端 key 列表（不含 terminal —— 那个始终可用）。
#[tauri::command]
fn detect_terminals() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let mut found = Vec::new();
        if Path::new("/Applications/iTerm.app").exists() {
            found.push("iterm2".to_string());
        }
        if Path::new("/Applications/Ghostty.app").exists() {
            found.push("ghostty".to_string());
        }
        if Path::new("/Applications/cmux.app").exists() {
            found.push("cmux".to_string());
        }
        if Path::new("/Applications/Warp.app").exists() {
            found.push("warp".to_string());
        }
        found
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[tauri::command]
fn add_bookmark(agent: String, path: String) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err("Directory does not exist".to_string());
    }
    bookmarks::add(&agent, &path)
}

#[tauri::command]
fn remove_bookmark(agent: String, path: String) -> Result<(), String> {
    bookmarks::remove(&agent, &path)
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---- CLI 环境检测 ----

#[tauri::command]
async fn check_cli_versions() -> Result<Vec<types::CliVersionInfo>, String> {
    tauri::async_runtime::spawn_blocking(cli_env::check_all_versions)
        .await
        .map_err(|e| format!("join: {e}"))
}

#[tauri::command]
async fn install_cli(cli_name: String) -> Result<types::CliUpgradeResult, String> {
    tauri::async_runtime::spawn_blocking(move || cli_env::install_single(&cli_name))
        .await
        .map_err(|e| format!("join: {e}"))?
}

#[tauri::command]
async fn upgrade_cli(cli_name: String) -> Result<types::CliUpgradeResult, String> {
    tauri::async_runtime::spawn_blocking(move || cli_env::upgrade_single(&cli_name))
        .await
        .map_err(|e| format!("join: {e}"))?
}

#[tauri::command]
async fn upgrade_all_clis() -> Result<Vec<types::CliUpgradeResult>, String> {
    tauri::async_runtime::spawn_blocking(cli_env::upgrade_all)
        .await
        .map_err(|e| format!("join: {e}"))?
}

#[tauri::command]
async fn diagnose_cli(cli_name: String) -> Result<types::CliDiagnosisResult, String> {
    tauri::async_runtime::spawn_blocking(move || cli_env::diagnose(&cli_name))
        .await
        .map_err(|e| format!("join: {e}"))?
}

#[tauri::command]
fn window_hide_to_tray(window: tauri::WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
fn window_exit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// 把原生窗口外观（标题栏 / 失焦时的红绿灯灰圈）同步到 App 内主题。
///
/// 此前原生外观只跟随 macOS 系统：浅色 App 主题下窗口失焦时，三个交通灯被画成浅灰，
/// 叠在同样浅色的自定义顶栏上几乎看不见（深色主题下灰圈对比够，所以正常）。把窗口
/// appearance 钉到当前主题后，深/浅两态都有正确对比。`theme=None`（系统模式）则交还
/// 系统自动跟随，避免破坏 webview 内 prefers-color-scheme 的自动切换。
#[tauri::command]
fn set_titlebar_theme(window: tauri::WebviewWindow, theme: Option<String>) {
    let t = match theme.as_deref() {
        Some("dark") => Some(tauri::Theme::Dark),
        Some("light") => Some(tauri::Theme::Light),
        _ => None,
    };
    let _ = window.set_theme(t);
}

/// 把字符串内容写到用户指定的绝对路径。
///
/// 历史：早期版本叫 save_to_downloads，自动落到 ~/Downloads；现在已经接入
/// tauri-plugin-dialog 的 save 对话框由前端拿到目标路径，所以后端只负责
/// 把字节安全写入指定位置。Tauri WKWebView 不支持 `<a download>`/blob URL，
/// 写盘必须经过 Rust。
#[tauri::command]
fn write_file(path: String, content: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
        }
    }
    fs::write(&p, content).map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(p.to_string_lossy().to_string())
}

#[tauri::command]
fn write_binary_file(path: String, base64: String) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&base64)
        .map_err(|e| format!("Invalid base64: {e}"))?;
    let p = PathBuf::from(&path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
        }
    }
    fs::write(&p, bytes).map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(p.to_string_lossy().to_string())
}

/// 把前端传来的 base64 图片数据保存到临时文件，返回路径。
/// 用于内嵌终端的 Cmd+V 贴图：xterm 拿到路径后写入 PTY stdin。
#[tauri::command]
fn save_clipboard_image(data: String, media_type: String) -> Result<String, String> {
    let ext = match media_type.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    };
    let ts = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    let name = format!("clipboard-{ts}.{ext}");
    let dir = std::env::temp_dir();
    let path = dir.join(&name);
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &data)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    fs::write(&path, &bytes).map_err(|e| format!("write failed: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// 在系统文件管理器中显示该文件。
#[tauri::command]
fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open Finder: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.replace('/', "\\")))
            .spawn()
            .map_err(|e| format!("Failed to open Explorer: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let parent = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(path);
        std::process::Command::new("xdg-open")
            .arg(&parent)
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {e}"))?;
    }
    Ok(())
}

/// 在系统默认浏览器中打开一个外部链接。只放行 http/https，避免 url 被
/// 当成本地文件或其它协议处理。
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("Only http(s) URLs are supported".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        crate::util::silent_command("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| format!("Failed to open URL: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {e}"))?;
    }
    Ok(())
}

/// macOS 上这些扩展名默认会被 Xcode 接管，按「文本方式」（`open -t` → 默认文本编辑器）打开更合适。
#[cfg(target_os = "macos")]
fn opens_as_text(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "mdx")
    )
}

/// 给定 app 内 CLI 的相对 bundle 路径（如 `Cursor.app/Contents/Resources/app/bin/cursor`），
/// 展开成 `/Applications` 与 `~/Applications` 两个候选绝对路径。GUI 进程 PATH 很薄，裸命令名
/// 往往找不到，所以直接查 bundle 里的固定 CLI 路径最可靠。
#[cfg(target_os = "macos")]
fn bundle_bins(rel: &str) -> Vec<String> {
    let mut v = vec![format!("/Applications/{rel}")];
    if let Some(home) = std::env::var_os("HOME") {
        v.push(format!("{}/Applications/{rel}", home.to_string_lossy()));
    }
    v
}

/// 源码 / 文本类文件点击时优先用「代码编辑器」打开 —— 而不是 macOS 默认那个（如 `.dart` 默认会被
/// Xcode 接管，又重又不对路）。pdf / 图片 / office / 压缩包等仍交给各自默认程序；无扩展名（含目录、
/// `Dockerfile` 这类）也走默认，交给 `open`。
#[cfg(target_os = "macos")]
fn wants_editor(p: &Path) -> bool {
    let ext = match p.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_ascii_lowercase(),
        None => return false,
    };
    matches!(
        ext.as_str(),
        // 代码
        "dart" | "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte"
            | "py" | "go" | "java" | "kt" | "kts" | "swift" | "m" | "mm" | "c" | "h"
            | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "cs" | "rb" | "php" | "scala"
            | "sh" | "bash" | "zsh" | "fish" | "ps1" | "lua" | "pl" | "r" | "sql"
            | "gradle" | "groovy"
            // 标记 / 配置 / 文本
            | "md" | "markdown" | "mdx" | "txt" | "json" | "jsonc" | "yaml" | "yml"
            | "toml" | "ini" | "cfg" | "conf" | "env" | "properties" | "xml" | "html"
            | "htm" | "css" | "scss" | "sass" | "less" | "csv" | "log"
    )
}

/// 用「代码编辑器」打开 `p`，有 `line`（可选 `col`）就定位到对应行。按常见度探测已装的编辑器，
/// 用第一个装了的：
///   - Trae（含国内版）/ VS Code / Cursor / Windsurf / VSCodium（VS Code 系）：CLI `-g file[:line[:col]]`，可跳行
///   - Zed / Sublime Text：CLI `file[:line[:col]]`，可跳行
///   - JetBrains（Android Studio / IntelliJ 系）：`open -a <bundle> <file>`，可靠打开但不跳行
///
/// 一个都没装 → 返回 `false`，调用方退回系统 `open`（用默认程序打开，不跳行）。
///
/// ⚠️ JetBrains 必须走 `open -a`，**绝不能**直接 exec 它的 `Contents/MacOS/<ide>` 主二进制：那不是
/// CLI，是 App 本体，直接跑会卡在单实例锁上 100% CPU 狂转、变成永不退出的僵尸进程（已踩坑）。
/// VS Code 系 / Zed / Sublime 的 `bin/*` 才是正经 CLI 包装器（连上 App 即退出），可以直接 spawn。
#[cfg(target_os = "macos")]
fn open_in_editor(p: &Path, line: Option<u32>, col: Option<u32>) -> bool {
    use std::process::Command;
    let file = p.to_string_lossy().into_owned();
    // `file:line:col` 形式（VS Code 系 / Zed / Sublime）；没行号就是裸路径。
    let goto = match (line, col) {
        (Some(l), Some(c)) => format!("{file}:{l}:{c}"),
        (Some(l), None) => format!("{file}:{l}"),
        (None, _) => file.clone(),
    };
    // VS Code 系用 `-g`；Zed / Sublime 直接吃 `file:line:col`。
    let code_args = match line {
        Some(_) => vec!["-g".to_string(), goto.clone()],
        None => vec![goto.clone()],
    };

    // ① 命令行工具型编辑器：bundle 里的 CLI（连上 App 后立即退出，安全）。Trae 放最前 —— 用户主力。
    let mut trae = bundle_bins("Trae.app/Contents/Resources/app/bin/trae");
    trae.extend(bundle_bins(
        "Trae CN.app/Contents/Resources/app/bin/trae-cn",
    ));
    trae.extend(bundle_bins("Trae CN.app/Contents/Resources/app/bin/trae"));
    let cli_editors: Vec<(Vec<String>, Vec<String>)> = vec![
        (trae, code_args.clone()),
        (
            bundle_bins("Visual Studio Code.app/Contents/Resources/app/bin/code"),
            code_args.clone(),
        ),
        (
            bundle_bins("Cursor.app/Contents/Resources/app/bin/cursor"),
            code_args.clone(),
        ),
        (
            bundle_bins("Windsurf.app/Contents/Resources/app/bin/windsurf"),
            code_args.clone(),
        ),
        (
            bundle_bins("VSCodium.app/Contents/Resources/app/bin/codium"),
            code_args.clone(),
        ),
        (
            bundle_bins("Zed.app/Contents/MacOS/cli"),
            vec![goto.clone()],
        ),
        (
            bundle_bins("Sublime Text.app/Contents/SharedSupport/bin/subl"),
            vec![goto.clone()],
        ),
    ];
    for (bins, args) in &cli_editors {
        if let Some(bin) = bins.iter().find(|b| Path::new(b).exists()) {
            if Command::new(bin).args(args).spawn().is_ok() {
                return true;
            }
        }
    }

    // ② JetBrains 系：用 `open -a <bundle> <file>` 交给 LaunchServices（可靠、不会狂转 CPU），但
    //    不支持跳行 —— 见上面 ⚠️。
    for app in ["Android Studio.app", "IntelliJ IDEA.app"] {
        if let Some(bundle) = bundle_bins(app).into_iter().find(|b| Path::new(b).exists()) {
            if Command::new("open")
                .arg("-a")
                .arg(&bundle)
                .arg(&file)
                .spawn()
                .is_ok()
            {
                return true;
            }
        }
    }
    false
}

/// 打开一个本地文件（聊天里 `@文件` / 文件引用 / Codex 文件附件的点击）。相对 / 部分路径按会话
/// `cwd` 解析。源码 / 文本类文件优先用代码编辑器打开（见 `wants_editor` / `open_in_editor`），
/// 避开 macOS 把 `.dart` 等交给 Xcode 的默认行为；传了 `line`（可选 `col`）还会跳到对应行。
/// 其它文件（pdf/图片/office…）及没装编辑器时，交给系统默认程序。只 spawn 进程、不经 shell，避免注入。
#[tauri::command]
fn open_path_external(
    path: String,
    cwd: Option<String>,
    line: Option<u32>,
    col: Option<u32>,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("Empty path".to_string());
    }
    let mut p = PathBuf::from(&path);
    if p.is_relative() {
        if let Some(base) = cwd.as_deref().filter(|c| !c.is_empty()) {
            p = Path::new(base).join(&p);
        }
    }
    if !p.exists() {
        if let Some(base) = cwd.as_deref().filter(|c| !c.is_empty()) {
            // 在 cwd 树里按目录段后缀搜（如 `bank/refund_detail.dart` → `lib/.../bank/refund_detail.dart`）
            if let Some(found) = util::resolve_file_ref(base, &path) {
                p = found;
            }
            // cwd 可能是子目录（如 src-tauri），往父目录逐级尝试
            if !p.exists() {
                let mut ancestor = Path::new(base).parent();
                while let Some(dir) = ancestor {
                    let candidate = dir.join(&path);
                    if candidate.exists() {
                        p = candidate;
                        break;
                    }
                    ancestor = dir.parent();
                }
            }
        }
    }
    if !p.exists() {
        return Err(format!("File not found: {}", p.to_string_lossy()));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (line, col); // 编辑器优先 / 跳行目前仅 macOS 实现；其它平台仅用默认程序打开。
    #[cfg(target_os = "macos")]
    {
        // 源码 / 文本类文件 → 优先用代码编辑器打开（有行号则跳到该行），避开默认把 .dart 等交给
        // Xcode 的行为。装了任一编辑器就到此为止；都没装才落回下面的系统 open。
        if wants_editor(&p) && open_in_editor(&p, line, col) {
            return Ok(());
        }
        // 没有可用编辑器时的兜底：.md 等纯文本用 `open -t` 走 LaunchServices 注册的默认文本编辑器
        // （一般 TextEdit），避开 Xcode；其它（xlsx/pdf/docx…）仍交给各自的默认程序。
        let mut cmd = std::process::Command::new("open");
        if opens_as_text(&p) {
            cmd.arg("-t");
        }
        cmd.arg(&p)
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        // start 需要一个空标题占位，否则带空格的路径会被当成窗口标题。
        crate::util::silent_command("cmd")
            .args(["/c", "start", ""])
            .arg(&p)
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&p)
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
    }
    Ok(())
}

/// 系统图片选择器给的是路径而非字节（没装 fs 插件），这里按路径读出来编成 base64，
/// 前端拿去做缩略图 + 视觉块（与粘贴/拖拽的图片同形）。仅用于图片附件。
#[tauri::command]
fn read_file_base64(path: String) -> Result<crate::types::ChatImageInput, String> {
    use base64::Engine as _;
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("File not found: {}", p.to_string_lossy()));
    }
    let bytes = std::fs::read(&p).map_err(|e| format!("Failed to read file: {e}"))?;
    Ok(crate::types::ChatImageInput {
        media_type: image_mime_from_ext(&p),
        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

/// 粘贴板图片无磁盘路径，存到临时目录供 Codex 等 agent 通过 @"path" 引用。
#[tauri::command]
fn save_temp_image(base64: String, media_type: String) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&base64)
        .map_err(|e| format!("base64 decode: {e}"))?;
    let ext = if media_type.contains("png") {
        "png"
    } else if media_type.contains("gif") {
        "gif"
    } else if media_type.contains("webp") {
        "webp"
    } else {
        "jpg"
    };
    let dir = std::env::temp_dir().join("cc-sessions-viewer-images");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
    let name = format!(
        "chat-img-{}.{ext}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let path = dir.join(&name);
    std::fs::write(&path, &bytes).map_err(|e| format!("write: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// GUI chat 附件：判断一个本地路径是否是目录。拖拽到输入框的路径可能是文件也可能是
/// 文件夹，前端据此决定 chip 用文件夹图标 +「打开文件夹」还是文件图标 +「打开文件」。
#[tauri::command]
fn path_is_dir(path: String) -> bool {
    Path::new(&path).is_dir()
}

/// chat 头部展示用：会话 `cwd` 所在仓库的当前分支名（无仓库 / 读不到 → None）。
#[tauri::command]
fn git_current_branch(cwd: String) -> Option<String> {
    util::git_current_branch(&cwd)
}

#[tauri::command]
fn git_repository_state(cwd: String) -> Result<crate::types::GitRepositoryState, String> {
    git::git_repository_state(&cwd)
}

#[tauri::command]
fn git_switch_branch(
    cwd: String,
    branch: String,
) -> Result<crate::types::GitRepositoryState, String> {
    git::git_switch_branch(&cwd, &branch)
}

#[tauri::command]
fn git_delete_branch(
    cwd: String,
    branch: String,
) -> Result<crate::types::GitRepositoryState, String> {
    git::git_delete_branch(&cwd, &branch)
}

#[tauri::command]
fn git_create_branch(
    cwd: String,
    branch: String,
) -> Result<crate::types::GitRepositoryState, String> {
    git::git_create_branch(&cwd, &branch)
}

#[tauri::command]
fn git_has_repo(cwd: String) -> bool {
    git::git_has_repo(&cwd)
}

#[tauri::command]
fn git_log(cwd: String, limit: Option<u32>) -> Result<Vec<crate::types::GitCommit>, String> {
    git::git_log(&cwd, limit)
}

#[tauri::command]
fn git_status(cwd: String) -> Result<Vec<crate::types::GitFileStatus>, String> {
    git::git_status(&cwd)
}

#[tauri::command]
fn git_diff_files(cwd: String, git_ref: String) -> Result<Vec<crate::types::GitDiffFile>, String> {
    git::git_diff_files(&cwd, &git_ref)
}

#[tauri::command]
fn git_diff_file(
    cwd: String,
    git_ref: String,
    path: String,
) -> Result<Vec<crate::types::DiffHunk>, String> {
    git::git_diff_file(&cwd, &git_ref, &path)
}

/// GUI chat 输入框 `@` 文件浮层：列出会话 `cwd` 下的目录/文件（相对路径）。`query` 空 →
/// 顶层直接子项；裸查询 → 全工作区模糊搜索；带路径查询 → 目录逐级浏览。
#[tauri::command]
async fn list_project_files(
    cwd: String,
    query: String,
    limit: usize,
) -> Vec<crate::types::ProjectFileEntry> {
    tauri::async_runtime::spawn_blocking(move || util::list_project_files(&cwd, &query, limit))
        .await
        .unwrap_or_default()
}

/// 由扩展名推断图片 MIME；未知回落到通用二进制类型。
fn image_mime_from_ext(p: &Path) -> String {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn path_env_candidates() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default()
}

fn find_in_path(bin: &str) -> Option<PathBuf> {
    let candidates = path_env_candidates();
    #[cfg(target_os = "windows")]
    let exts: Vec<OsString> = std::env::var_os("PATHEXT")
        .map(|v| {
            v.to_string_lossy()
                .split(';')
                .filter(|s| !s.is_empty())
                .map(OsString::from)
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                OsString::from(".exe"),
                OsString::from(".cmd"),
                OsString::from(".bat"),
            ]
        });
    #[cfg(not(target_os = "windows"))]
    let exts: Vec<OsString> = vec![OsString::new()];

    for dir in candidates {
        for ext in &exts {
            let cand = if ext.is_empty() {
                dir.join(bin)
            } else {
                dir.join(format!("{bin}{}", ext.to_string_lossy()))
            };
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

fn parse_local_target(input: &str) -> (String, Option<u32>, Option<u32>) {
    let trimmed = input.trim();
    let parts: Vec<&str> = trimmed.rsplitn(3, ':').collect();
    if parts.len() >= 2 {
        if let Ok(last) = parts[0].parse::<u32>() {
            let base_one = parts[1..]
                .iter()
                .rev()
                .copied()
                .collect::<Vec<_>>()
                .join(":");
            let base_one_path = Path::new(&base_one);
            if base_one_path.is_absolute() || base_one_path.exists() {
                if parts.len() == 3 {
                    if let Ok(mid) = parts[1].parse::<u32>() {
                        let base_two = parts[2];
                        let base_two_path = Path::new(base_two);
                        if base_two_path.is_absolute() || base_two_path.exists() {
                            return (base_two.to_string(), Some(mid), Some(last));
                        }
                    }
                }
                return (base_one, Some(last), None);
            }
        }
    }
    (trimmed.to_string(), None, None)
}

fn open_with_editor(path: &str, line: Option<u32>, column: Option<u32>) -> Result<bool, String> {
    let Some(line) = line else {
        return Ok(false);
    };
    let column = column.unwrap_or(1);

    let specs: &[(&str, &[&str])] = &[
        ("cursor", &["-g"]),
        ("code", &["-g"]),
        ("code-insiders", &["-g"]),
        ("codium", &["-g"]),
        ("windsurf", &["-g"]),
        ("zed", &[]),
        ("subl", &[]),
        ("mate", &["-l"]),
        ("bbedit", &[]),
    ];

    for (bin, prefix) in specs {
        let Some(found) = find_in_path(bin) else {
            continue;
        };
        // Windows 上 code/cursor 等命令是 .cmd 批处理垫片，直接 spawn 会闪控制台窗口。
        let mut cmd = crate::util::silent_command(found);
        for arg in *prefix {
            cmd.arg(arg);
        }
        match *bin {
            "mate" => {
                cmd.arg(line.to_string()).arg(path);
            }
            "bbedit" => {
                cmd.arg(format!("+{line}")).arg(path);
            }
            "zed" | "subl" => {
                cmd.arg(format!("{path}:{line}:{column}"));
            }
            _ => {
                cmd.arg(format!("{path}:{line}:{column}"));
            }
        }
        match cmd.spawn() {
            Ok(_) => return Ok(true),
            Err(err) => return Err(format!("Failed to launch editor: {err}")),
        }
    }
    Ok(false)
}

/// 打开本地文件。若 path 形如 `/abs/file:12:3`，会尽量让编辑器定位到该行列。
#[tauri::command]
fn open_local_path(path: String) -> Result<(), String> {
    let (file_path, line, column) = parse_local_target(&path);
    let target = PathBuf::from(&file_path);

    if !target.is_absolute() {
        return Err("Only absolute paths are supported".to_string());
    }

    if open_with_editor(&file_path, line, column)? {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("Failed to open local file: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        crate::util::silent_command("cmd")
            .args(["/c", "start", "", &file_path])
            .spawn()
            .map_err(|e| format!("Failed to open local file: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("Failed to open local file: {e}"))?;
    }
    Ok(())
}

/// 手动从 models.dev 上游拉一次模型价格表，覆盖本地 24h 缓存。前端 Settings
/// 「立即刷新模型价格」按钮调用。返回入表条数；失败返回错误字符串（前端弹 toast）。
///
/// **必须是 async**：内部 `refresh_blocking` 走 `ureq::get(...).call()`，是真同步阻塞
/// 调用，timeout 高达 20s。如果当 sync Tauri 命令直接跑，会霸占 webview 主线程，
/// UI 一切动画 / 滚动 / 鼠标光标全冻 —— 用户反馈"点了刷新像卡死了"就是这个。
/// 改成 async + `spawn_blocking` 后阻塞活路扔进 Tauri 的后台线程池，UI 线程立刻
/// 返回继续跑 CSS 动画，等结果时 webview 仍然响应。
#[tauri::command]
async fn refresh_pricing() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(stats::pricing::refresh_blocking)
        .await
        .map_err(|e| format!("join: {e}"))?
}

/// 价格表当前状态。前端按 `loaded` / `fetching` / `lastError` 决定渲染：
///   - loaded=false && fetching=true → 显示加载占位
///   - loaded=false && lastError=Some → 显示 error placeholder
///   - loaded=true → 正常渲染（即使过期 cache 也先用着）
#[tauri::command]
fn pricing_status() -> stats::pricing::PricingStatus {
    stats::pricing::status()
}

/// 返回当前价格表里 Claude / Codex 两家的全部模型 —— 给 PricingView 弹窗渲染。
/// 已按 family 分组、组内按 input 单价升序，前端可直接 group_by(family) 渲染。
#[tauri::command]
fn list_pricing() -> Vec<stats::pricing::PricingEntry> {
    stats::pricing::list_for_ui()
}

/// 账号额度（5 小时 / 周 / 各模型分项）—— 走 Claude OAuth 用量接口，返回每个窗口的
/// 精确利用率 + 重置时间。前端底栏据此渲染 5h / 周徽标（随时精确，不依赖越阈值事件）。
/// async + spawn_blocking：内部 `curl` 子进程是同步阻塞，不能霸占 webview 主线程。
/// `force=true`：绕过 20s 缓存强制拉新（事件驱动刷新 —— 一轮对话结束后用，确保拿到刚变化的值）。
#[tauri::command]
async fn account_usage(force: Option<bool>) -> Result<usage_api::AccountUsage, String> {
    let force = force.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || usage_api::account_usage_blocking(force))
        .await
        .map_err(|e| format!("join: {e}"))?
}

/// 托盘弹窗用的快速统计：一次扫描三个时间窗口，返回 per-agent 的 token + cost。
/// async + spawn_blocking —— 扫描耗时取决于会话数量（几百毫秒到几秒），不能阻塞主线程。
#[tauri::command]
async fn tray_quick_stats() -> Result<TrayStats, String> {
    tauri::async_runtime::spawn_blocking(stats::tray::quick_stats)
        .await
        .map_err(|e| format!("join: {e}"))?
}

/// Attach an empty `NSToolbar` with `unifiedCompact` style so AppKit grows the
/// titlebar to ~40px and auto-centers the traffic lights vertically inside it
/// — matching our 40px CSS topbar. This is the SUPPORTED AppKit way to extend
/// the titlebar; manually `setFrameOrigin`-ing the standardWindowButtons works
/// visually but appears to confuse AppKit's titlebar drag tracking (focused
/// click→drag stops working).
#[cfg(target_os = "macos")]
fn pin_traffic_lights(window: &tauri::WebviewWindow) {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSToolbar, NSWindow, NSWindowToolbarStyle};

    let ns_window_ptr = match window.ns_window() {
        Ok(p) => p as *mut AnyObject,
        Err(_) => return,
    };
    if ns_window_ptr.is_null() {
        return;
    }

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    unsafe {
        let ns_window: Retained<NSWindow> = match Retained::retain(ns_window_ptr.cast::<NSWindow>())
        {
            Some(w) => w,
            None => return,
        };
        if ns_window.toolbar().is_some() {
            return; // 已挂好，避免重复
        }
        let toolbar = NSToolbar::new(mtm);
        ns_window.setToolbar(Some(&toolbar));
        ns_window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    panic_log::install();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build());

    // 开发期注入 MCP Bridge —— 让 AI 助手经 WebSocket 直接看/控这个 app（截图 /
    // DOM 快照 / 执行 JS / 监控 IPC）。feature "dev-mcp"（default 但 release
    // 构建通过 --no-default-features 排除）控制是否编译链接。
    // 绑 127.0.0.1（默认是 0.0.0.0），避免把调试端口 9223 暴露到局域网。
    #[cfg(feature = "dev-mcp")]
    let builder = builder.plugin(
        tauri_plugin_mcp_bridge::Builder::new()
            .bind_address("127.0.0.1")
            .build(),
    );

    builder
        .invoke_handler(tauri::generate_handler![
            list_projects,
            list_sessions,
            read_session,
            watch_session,
            unwatch_session,
            check_watched_session,
            terminal_turn_signal,
            install_turn_hooks,
            turn_hook_status,
            desktop_pet_tasks,
            acknowledge_desktop_pet_task,
            resolve_desktop_pet_session,
            desktop_pet_assets::desktop_pet_catalog,
            desktop_pet_assets::delete_custom_desktop_pet,
            set_desktop_pet_enabled,
            focus_desktop_pet_main,
            open_desktop_pet_session,
            claude_runtime_info,
            codex_runtime_info,
            session_usage,
            session_last_prompt,
            session_context_usage,
            agent_stats,
            start_agent_stats,
            cancel_stats,
            search_sessions,
            cancel_search,
            rename_session,
            fork_session,
            fork_session_before_user_turn,
            purge_btw_session,
            codex_archive_session,
            soft_delete_session,
            hard_delete_session,
            create_worktree,
            remove_worktree,
            cleanup_worktree_project_dirs,
            list_trash,
            restore_session,
            permanent_delete_trash,
            empty_trash,
            resume_session,
            new_session,
            detect_terminals,
            pty_spawn,
            pty_spawn_new,
            pty_spawn_shell,
            pty_write,
            pty_resize,
            pty_kill,
            agent_chat_start,
            agent_chat_list_running,
            agent_chat_send,
            agent_chat_steer,
            agent_chat_fork_before_last_turn,
            agent_chat_stop,
            agent_chat_set_title,
            agent_chat_interrupt,
            agent_chat_respond_permission,
            agent_chat_respond_question,
            reclaude_info,
            agent_chat_slash_commands,
            reveal_in_finder,
            open_local_path,
            open_url,
            open_path_external,
            read_file_base64,
            save_temp_image,
            save_clipboard_image,
            path_is_dir,
            git_current_branch,
            git_repository_state,
            git_switch_branch,
            git_delete_branch,
            git_create_branch,
            git_has_repo,
            git_log,
            git_status,
            git_diff_files,
            git_diff_file,
            list_project_files,
            write_file,
            write_binary_file,
            background_media::background_media_directory,
            background_media::list_background_media,
            background_media::import_background_media,
            background_media::delete_background_media,
            set_titlebar_theme,
            add_bookmark,
            remove_bookmark,
            app_version,
            window_hide_to_tray,
            window_exit_app,
            refresh_pricing,
            pricing_status,
            list_pricing,
            account_usage,
            tray_quick_stats,
            check_cli_versions,
            install_cli,
            upgrade_cli,
            upgrade_all_clis,
            diagnose_cli,
        ])
        .setup(|app| {
            // 启动期后台拉一次 models.dev 模型价格表，新模型上架不必发版。
            // 不阻塞 setup —— init() 自己 spawn 后台线程，离线 / 失败时先用过期
            // 磁盘缓存兜着，前端按 pricing_status 渲染 error placeholder。
            stats::pricing::init();
            if let Err(e) = turn::start_signal_watcher(app.handle().clone()) {
                eprintln!("turn signal watcher failed: {e}");
            }

            #[cfg(target_os = "windows")]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_decorations(false);
                    let win_clone = win.clone();
                    win.on_window_event(move |e| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                            api.prevent_close();
                            let _ = win_clone.emit("window://close-requested", ());
                        }
                    });
                }
                tray_windows::build(app.handle())?;
            }

            #[cfg(target_os = "macos")]
            {
                // 原生应用菜单只保留给 macOS。Windows 的窗口内菜单栏会挤占
                // 自定义顶栏，视觉上和 WebView command bar 重复。
                menu::build(app.handle())?;
                menu::install_bridges(app.handle());

                // 菜单栏托盘图标 + 菜单（Show / Settings / Quit）。
                tray::build(app.handle())?;

                if let Some(win) = app.get_webview_window("main") {
                    pin_traffic_lights(&win);
                    // AppKit relays out standard window buttons on resize,
                    // so re-pin then. Avoid Focused / ThemeChanged: AppKit
                    // does NOT recreate the buttons on those events, and
                    // running Objective-C work inside the Focused handler
                    // can race the click→drag transition and break titlebar
                    // dragging when focusing the window from a click.
                    let win_clone = win.clone();
                    win.on_window_event(move |e| match e {
                        tauri::WindowEvent::Resized(_) => pin_traffic_lights(&win_clone),
                        // Close-to-tray：红灯 / ⌘W 不退出，藏到菜单栏，仍可从托盘
                        // "Show" 唤回；真正退出走托盘 "Quit" 或 ⌘Q。
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            let _ = win_clone.hide();
                        }
                        _ => {}
                    });
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run({
            let exiting = std::sync::atomic::AtomicBool::new(false);
            move |app, event| {
                // Dock 图标点击（macOS Reopen）：close-to-tray 把窗口藏起来后，点 Dock
                // 图标应能唤回它，否则只能从托盘菜单 "Show"。
                #[cfg(target_os = "macos")]
                if let tauri::RunEvent::Reopen { .. } = event {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
                // 退出拦截：所有退出路径（X→退出的 window_exit_app、Windows 托盘 Quit、
                // macOS ⌘Q / 托盘 terminate:）都先到这里。webview 的 localStorage 是
                // 异步刷盘的，直接 exit 会硬杀 WebView2/WKWebView，丢掉最近几秒写入
                // （表现为重开后 tab 恢复不全）。第一次到达时拦下：通知前端保存 tab
                // 状态 → 销毁窗口让 webview 控制器干净关闭（这一步才会触发刷盘）→
                // 再放行真正退出。
                if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                    use std::sync::atomic::Ordering;
                    if exiting.swap(true, Ordering::SeqCst) {
                        return; // 第二次 ExitRequested（下面 thread 里触发的）→ 放行
                    }
                    api.prevent_exit();
                    let _ = app.emit("app://before-quit", ());
                    let app = app.clone();
                    std::thread::spawn(move || {
                        // 给前端 before-quit 处理器一拍时间把状态写进 localStorage
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        for (_, w) in app.webview_windows() {
                            let _ = w.destroy();
                        }
                        // 窗口销毁后浏览器进程开始正常收尾刷盘；稍等再退，避免抢跑
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        app.exit(code.unwrap_or(0));
                    });
                }
            }
        });
}
