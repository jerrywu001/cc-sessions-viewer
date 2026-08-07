// Claude Code 会话源：~/.claude/projects/<dir>/<sessionId>.jsonl
//
// 每行是 `{ "type": "user" | "assistant" | "custom-title" | ..., ... }`，
// user/assistant 的 `message.content` 数组里夹着 text / thinking / tool_use /
// tool_result / image 等块。

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rayon::prelude::*;
use serde_json::Value;

use super::{ChatEvent, SessionSource};
use crate::agent_command::AgentCommand;
use crate::stats::{
    pricing, shell as shell_util,
    types::{CallRecord, Turn},
};
use crate::types::{
    Block, ChatDelta, DiffHunk, DiffLine, Msg, ProjectInfo, SessionMeta, SessionPage, UsageSummary,
};
use crate::util::{
    append_jsonl_line, clean_title, home, inline_at_file_path, is_jsonl, mtime_millis,
    parse_iso8601_ms, strip_edge_references, text_block, validate_rename_name,
};

pub struct ClaudeSource;

fn projects_dir() -> PathBuf {
    home().join(".claude").join("projects")
}

fn list_projects_in(dir: &Path) -> Result<Vec<ProjectInfo>, String> {
    let mut out = Vec::new();
    // ~/.claude/projects/ 不存在（刚装 CLI 还没跑过任何会话）→ 返回空列表，
    // 不要报错，否则后续 bookmark 逻辑全部跳过、侧栏为空。
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(format!("Failed to read project directory: {e}")),
    };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = e.file_name().to_string_lossy().to_string();
        let mut count = 0usize;
        let mut last = 0u64;
        let mut cwd: Option<String> = None;
        if let Ok(files) = fs::read_dir(&path) {
            for f in files.flatten() {
                let fp = f.path();
                if is_jsonl(&fp) {
                    count += 1;
                    let m = mtime_millis(&fp);
                    if m > last {
                        last = m;
                    }
                    if cwd.is_none() {
                        cwd = last_cwd(&fp);
                    }
                }
            }
        }
        if count == 0 {
            continue;
        }
        let display_path = cwd.unwrap_or_else(|| dir_name.replace('-', "/"));
        let display_path = best_project_root(&dir_name, &display_path);
        let exists = Path::new(&display_path).is_dir();
        let (parent_dir_name, worktree_name) =
            if let Some(pos) = dir_name.find("--claude-worktrees-") {
                let parent = dir_name[..pos].to_string();
                let wt = dir_name[pos + "--claude-worktrees-".len()..].to_string();
                (Some(parent), Some(wt))
            } else {
                (None, None)
            };
        out.push(ProjectInfo {
            dir_name,
            display_path,
            session_count: count,
            last_modified: last,
            exists,
            bookmarked: false,
            parent_dir_name,
            worktree_name,
        });
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.last_modified));
    Ok(out)
}

impl SessionSource for ClaudeSource {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn list_projects(
        &self,
        _include_codex_internal: bool,
        _include_codex_archived: bool,
    ) -> Result<Vec<ProjectInfo>, String> {
        list_projects_in(&projects_dir())
    }

    fn list_sessions(
        &self,
        project_key: &str,
        offset: usize,
        limit: usize,
        _include_codex_internal: bool,
        _include_codex_archived: bool,
    ) -> Result<SessionPage, String> {
        let pdir = projects_dir().join(project_key);
        let mut files: Vec<(PathBuf, u64)> = Vec::new();
        let entries =
            fs::read_dir(&pdir).map_err(|e| format!("Failed to read session directory: {e}"))?;
        for f in entries.flatten() {
            let fp = f.path();
            if is_jsonl(&fp) {
                let mt = mtime_millis(&fp);
                files.push((fp, mt));
            }
        }
        files.sort_by_key(|f| std::cmp::Reverse(f.1));
        let total = files.len();
        // 本页要扫的文件可能各自几十 MB，串行 scan 一个大项目要 ~10s。scan 是纯 CPU（读+切分），
        // 用 rayon 铺到多核并行；scan 内部的缓存锁只在 get/insert 时短暂持有，不会成为瓶颈。
        // par_iter().collect() 保序，列表顺序不变。
        let window: Vec<&PathBuf> = files
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(p, _)| p)
            .collect();
        let sessions: Vec<SessionMeta> = window.par_iter().map(|p| scan(p)).collect();
        Ok(SessionPage { total, sessions })
    }

    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String> {
        let mut msgs = read(path)?;
        crate::util::post_process_session_msgs(&mut msgs);
        Ok(msgs)
    }

    fn discover_stats_sessions(&self, project_key: &str) -> Result<Vec<SessionMeta>, String> {
        let pdir = projects_dir().join(project_key);
        let mut out: Vec<SessionMeta> = Vec::new();
        let entries =
            fs::read_dir(&pdir).map_err(|e| format!("Failed to read session directory: {e}"))?;
        for f in entries.flatten() {
            let path = f.path();
            if is_jsonl(&path) {
                out.push(scan(&path));
                continue;
            }
            // <sessionId>/subagents/*.jsonl —— 子代理产生的独立 JSONL，
            // 是真实的 API 调用且独立计费。codeburn 用同名 collectJsonlFiles 逻辑。
            // 不进 list_sessions（避免污染聊天列表），只进统计扫描。
            if path.is_dir() {
                let sub = path.join("subagents");
                if let Ok(sub_entries) = fs::read_dir(&sub) {
                    for sf in sub_entries.flatten() {
                        let sp = sf.path();
                        if is_jsonl(&sp) {
                            out.push(scan(&sp));
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// 单会话同伴文件：`<projects>/<projectKey>/<sessionId>.jsonl` 的旁边可能
    /// 有 `<projects>/<projectKey>/<sessionId>/subagents/*.jsonl`。把它们也算入
    /// 单会话统计，跟全局 by-session 的口径一致（codeburn 同样做法）。
    fn discover_session_companions(&self, path: &str) -> Vec<SessionMeta> {
        let parent_path = Path::new(path);
        // parent.with_extension("") -> "<projects>/<projectKey>/<sessionId>"
        let sub_dir = parent_path.with_extension("").join("subagents");
        let Ok(entries) = fs::read_dir(&sub_dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for sf in entries.flatten() {
            let sp = sf.path();
            if is_jsonl(&sp) {
                out.push(scan(&sp));
            }
        }
        out
    }

    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String> {
        let trimmed = validate_rename_name(name)?;
        let id = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.trim_end_matches(".jsonl").to_string())
            .unwrap_or_default();
        // Claude Code `/rename` 会成对追加 custom-title + agent-name 两条记录
        // （同值）。这里照搬，保证 claude CLI 与本应用互认。
        let title_line = serde_json::json!({
            "type": "custom-title",
            "customTitle": trimmed,
            "sessionId": id,
        })
        .to_string();
        let agent_line = serde_json::json!({
            "type": "agent-name",
            "agentName": trimmed,
            "sessionId": id,
        })
        .to_string();
        append_jsonl_line(path, &title_line)?;
        append_jsonl_line(path, &agent_line)?;
        // 运行时镜像：若该会话当前有运行中的 claude 进程，更新对应 PID.json
        // 的 name。是 best-effort，找不到 / 失败都不影响持久标题。
        mirror_runtime_name(&id, trimmed);
        Ok(())
    }

    fn fork_session(
        &self,
        project_key: &str,
        source_id: &str,
        title: &str,
    ) -> Result<String, String> {
        fork_session(project_key, source_id, title)
    }

    fn fork_session_before_user_turn(
        &self,
        project_key: &str,
        source_id: &str,
        title: &str,
        keep_user_turns: usize,
    ) -> Result<String, String> {
        fork_session_before_user_turn(project_key, source_id, title, keep_user_turns)
    }

    fn trash_title(&self, path: &Path) -> String {
        scan(path).title
    }

    fn resume_command(&self, session_id: &str, _path: &str) -> AgentCommand {
        AgentCommand::new("claude").arg("--resume").arg(session_id)
    }

    fn new_session_command(&self) -> AgentCommand {
        AgentCommand::new("claude")
    }

    /// headless stream-json：管道驱动 + 逐行事件。`-p`（print）配合
    /// `--input-format stream-json` 让 claude 从 stdin 持续读 JSON 用户消息、保持长驻；
    /// `--output-format stream-json --verbose` 让它把 system/assistant/user/result 事件
    /// 逐行吐到 stdout。`--resume <id>` 续聊既有会话。
    fn chat_command(
        &self,
        session_id: Option<&str>,
        permission_mode: &str,
        model: Option<&str>,
        effort: Option<&str>,
        fork: bool,
    ) -> Option<AgentCommand> {
        let mut cmd = AgentCommand::new("claude")
            .arg("--print")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            // token 级流式：额外吐 `stream_event`（content_block_delta 等）；
            // 权威 `assistant` 记录仍随后到达，故只是叠加、不破坏现有解析。
            .arg("--include-partial-messages")
            // 交互式工具审批：`stdio` 是哨兵值（非 MCP 工具名），让被门控的工具走控制协议
            // 发 `can_use_tool` 请求到 stdout，由 GUI 弹框、用户决定后经 stdin 回 `control_response`。
            // 与 `--permission-mode` 正交：default 全程问、acceptEdits 仅放行编辑（Bash 等仍问）、
            // bypassPermissions 全程不问（不产生该请求）。
            .arg("--permission-prompt-tool")
            .arg("stdio")
            .arg("--permission-mode")
            .arg(permission_mode);
        // model 为别名（opus / sonnet / haiku / fable）或全名；effort 取
        // low|medium|high|xhigh|max。None 走 CLI 默认（不下发 flag）。长驻进程下这两者
        // 在 start 时定型，切换靠 restart-with-resume。
        if let Some(m) = model {
            cmd = cmd.arg("--model").arg(m);
        }
        if let Some(e) = effort {
            cmd = cmd.arg("--effort").arg(e);
        }
        if let Some(id) = session_id {
            cmd = cmd.arg("--resume").arg(id);
            // btw 侧聊：派生新 session id（继承上下文、不续写原 transcript）。仅在续聊既有
            // 会话时有意义；新开会话（session_id 为空）无可派生对象，忽略 fork。
            if fork {
                cmd = cmd.arg("--fork-session");
            }
        }
        Some(cmd)
    }

    fn parse_chat_line(&self, line: &str) -> ChatEvent {
        parse_chat_line(line)
    }

    fn chat_slash_commands(&self, cwd: &str) -> Vec<crate::types::SlashCommand> {
        chat_slash_commands(cwd)
    }

    fn image_src(&self, block: &Value) -> Option<String> {
        image_src(block)
    }

    fn usage_summary(&self, path: &str) -> Result<UsageSummary, String> {
        usage_summary(Path::new(path))
    }

    fn context_usage(&self, path: &str) -> Result<UsageSummary, String> {
        Ok(last_context_usage(Path::new(path)))
    }

    fn last_prompt(&self, path: &str) -> Result<Option<String>, String> {
        Ok(last_user_text(Path::new(path)))
    }

    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String> {
        Ok(read_turns(Path::new(path)))
    }
}

// ----- 内部解析 --------------------------------------------------------------

/// 从 JSONL 尾部反向读，找最后一条 `role: human` 的文本。
fn last_user_text(fp: &Path) -> Option<String> {
    let raw = fs::read(fp).ok()?;
    // 反向逐行扫描
    for line in raw.rsplit(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let content = v.get("message").and_then(|m| m.get("content"));
        let text = content
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|c| c.get("type").and_then(Value::as_str) == Some("text"))
            })
            .and_then(|c| c.get("text").and_then(Value::as_str))
            .or_else(|| content.and_then(Value::as_str));
        if let Some(t) = text {
            let clean = crate::util::truncate_subtitle(t);
            if !clean.is_empty() {
                return Some(clean);
            }
        }
    }
    None
}

/// 一次性把整份 JSONL 走一遍，累加每条 assistant 消息里的 `message.usage` 字段。
/// Claude 的形状：
///   {"type":"assistant","message":{"usage":{"input_tokens":N, "output_tokens":N,
///       "cache_creation_input_tokens":N, "cache_read_input_tokens":N, ...}}}
/// user 消息没有 usage；不存在的字段当 0 处理。文件不可读 → 返回 default 而非
/// 错误，避免会话列表里因为一个坏文件整个挂掉 —— 用户看到「0 tokens」也比看到
/// 全列表挂掉好。
fn usage_summary(fp: &Path) -> Result<UsageSummary, String> {
    let file = match fs::File::open(fp) {
        Ok(f) => f,
        Err(_) => return Ok(UsageSummary::default()),
    };
    let mut acc = UsageSummary::default();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let usage = v
            .get("message")
            .and_then(|m| m.get("usage"))
            .or_else(|| v.get("usage"));
        let Some(u) = usage else { continue };
        acc.input_tokens += u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        acc.output_tokens += u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        acc.cache_creation_input_tokens += u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        acc.cache_read_input_tokens += u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    Ok(acc.finalize())
}

/// 取文件里**最后一条**带非零 usage 的记录 = 会话末尾喂给模型的上下文规模。
/// 区别于 `usage_summary` 的全程累加：这里只保留最近一条（不累加），用作 resume
/// 后的「当前上下文」种子值。全 0 的 usage（如 user 消息、占位）跳过，避免把末尾
/// 一条没意义的零值当成上下文。文件不可读 → default。
fn last_context_usage(fp: &Path) -> UsageSummary {
    let file = match fs::File::open(fp) {
        Ok(f) => f,
        Err(_) => return UsageSummary::default(),
    };
    let mut last = UsageSummary::default();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(u) = v
            .get("message")
            .and_then(|m| m.get("usage"))
            .or_else(|| v.get("usage"))
        else {
            continue;
        };
        let input = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        let cache_creation = u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cache_read = u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        // 跳过没有上下文输入的 usage（纯输出 / 占位 / user 行）
        if input + cache_creation + cache_read == 0 {
            continue;
        }
        let cur = UsageSummary {
            input_tokens: input,
            output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
            cache_creation_input_tokens: cache_creation,
            cache_read_input_tokens: cache_read,
            ..Default::default()
        };
        last = cur.finalize();
    }
    last
}

fn best_project_root(dir_name: &str, cwd: &str) -> String {
    let mut path = Path::new(cwd);
    loop {
        let encoded = format!(
            "-{}",
            path.to_string_lossy()
                .trim_start_matches('/')
                .replace('/', "-")
        );
        if encoded == dir_name {
            return path.to_string_lossy().into_owned();
        }
        match path.parent() {
            Some(p) if p != path => path = p,
            _ => break,
        }
    }
    cwd.to_string()
}

fn last_cwd(fp: &Path) -> Option<String> {
    let file = fs::File::open(fp).ok()?;
    let mut last: Option<String> = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                last = Some(c.to_string());
            }
        }
    }
    last
}

/// 用户在 Claude 处理过程中排队输入的消息会被记成
/// `{"type":"attachment","attachment":{"type":"queued_command","prompt":...}}`，
/// 而非常规的 `type:"user"` 记录。把其中的 `prompt` 解析成消息块：纯文本排队
/// 消息的 `prompt` 是字符串，带贴图的则是 text / image 块数组。非排队命令的
/// attachment（hook_success / task_reminder / diagnostics 等）返回 None。
fn queued_command_blocks(v: &Value) -> Option<Vec<Block>> {
    let att = v.get("attachment")?;
    if att.get("type").and_then(|x| x.as_str()) != Some("queued_command") {
        return None;
    }
    let mut blocks = Vec::new();
    match att.get("prompt")? {
        Value::String(s) if !s.trim().is_empty() => {
            blocks.push(text_block("text", s));
        }
        Value::Array(arr) => {
            for el in arr {
                match el.get("type").and_then(|x| x.as_str()) {
                    Some("text") => {
                        if let Some(s) = el.get("text").and_then(|x| x.as_str()) {
                            if !s.trim().is_empty() {
                                blocks.push(text_block("text", s));
                            }
                        }
                    }
                    Some("image") => {
                        if let Some(src) = image_src(el) {
                            blocks.push(Block {
                                kind: "image".to_string(),
                                image_src: Some(src),
                                ..Default::default()
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

fn user_text(v: &Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => {
            for el in arr {
                if el.get("type").and_then(|x| x.as_str()) == Some("text") {
                    if let Some(s) = el.get("text").and_then(|x| x.as_str()) {
                        return Some(s.to_string());
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Claude: `{"type":"image","source":{"type":"base64"|"url", ...}}`
fn image_src(el: &Value) -> Option<String> {
    if el.get("type").and_then(|x| x.as_str()) != Some("image") {
        return None;
    }
    let source = el.get("source")?;
    let src_type = source.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if src_type == "base64" {
        let media = source
            .get("media_type")
            .and_then(|x| x.as_str())
            .unwrap_or("image/png");
        let data = source.get("data").and_then(|x| x.as_str())?;
        return Some(format!("data:{media};base64,{data}"));
    }
    if src_type == "url" {
        return source
            .get("url")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
    }
    None
}

/// 判断这条 user 消息是不是 Claude Code 紧跟在真实贴图之后写下的图片元引用，
/// 形如 `[Image: source: <local-path>]` 或 `[Image: original WxH, displayed at ...]`。
/// 真正的贴图已经在上一条 user 记录里以 base64 渲染过了，这种纯元数据直接丢弃。
/// 一条 user 记录可能携带多张图（content 数组里多个 text block），只要全是这类
/// 元引用就整体跳过。
fn is_image_source_meta(v: &Value, blocks: &[Block]) -> bool {
    let is_meta = v.get("isMeta").and_then(|x| x.as_bool()).unwrap_or(false);
    if !is_meta {
        return false;
    }
    if blocks.is_empty() {
        return false;
    }
    blocks.iter().all(|b| {
        if b.kind != "text" {
            return false;
        }
        let txt = b.text.as_deref().unwrap_or("").trim();
        if !txt.starts_with("[Image:") || !txt.ends_with(']') {
            return false;
        }
        let inner = txt.trim_start_matches("[Image:").trim_start();
        inner.starts_with("source:") || inner.starts_with("original")
    })
}

/// 解析用户文本里 Claude Code 的 `@文件` 引用：拖拽文件 / 用 `@` 选文件时，CC 会把形如
/// `@"/abs/path with space.ext"`（带引号）或 `@/abs/path.ext`、`@dir/rel.ext`（不带引号、
/// 不含空白）的标记写进 JSONL 原文。每个引用都会抽成 `file` 块（前端渲染成可点击外部打开的
/// 文件 chip）；只有提示词首尾连续的附件区会从正文剔除。中间引用同时保留 tag 和完整原文，
/// 避免「分析@file.sh的执行过程」被截断。返回 (file 块, 正文)。
fn extract_file_refs(text: &str, cwd: Option<&Path>) -> (Vec<Block>, String) {
    use regex_lite::Regex;
    let re = Regex::new(r#"@"([^"]+)"|@(\S+)"#).expect("valid file-ref regex");
    let mut files = Vec::new();
    let mut refs = Vec::new();
    for caps in re.captures_iter(text) {
        let whole = caps.get(0).unwrap();
        let (path, reference_end) = match (caps.get(1), caps.get(2)) {
            (Some(q), _) => (Some(q.as_str().to_string()), whole.end()),
            (None, Some(u)) => {
                let path = inline_at_file_path(u.as_str(), cwd);
                let end = path
                    .as_ref()
                    .map(|path| whole.start() + 1 + path.len())
                    .unwrap_or(whole.end());
                (path, end)
            }
            _ => (None, whole.end()),
        };
        // path 为 None（普通 @提及，如 @某人）：不剔除，留给后续 cleaned 原样保留。
        if let Some(p) = path {
            // stat 一次区分文件 / 文件夹，让历史 chip 与实时回显一样显示对的图标 + 提示。
            // 仅确为目录才标 Some(true)；文件 / 解析不出（相对路径等）留 None → 文件图标。
            let path_on_disk = if Path::new(&p).is_absolute() {
                PathBuf::from(&p)
            } else if let Some(cwd) = cwd {
                cwd.join(&p)
            } else {
                PathBuf::from(&p)
            };
            let is_dir = path_on_disk.is_dir().then_some(true);
            files.push(Block {
                kind: "file".to_string(),
                file_path: Some(p),
                is_dir,
                ..Default::default()
            });
            refs.push((whole.start(), reference_end));
        }
    }
    (files, tidy_after_strip(&strip_edge_references(text, &refs)))
}

/// 删掉引用后收尾：逐行去行尾空白、3+ 连续换行收敛成 2、整体 trim。文件引用多半独占一行，
/// 删掉后会留下空行，这里一并清掉。
fn tidy_after_strip(s: &str) -> String {
    let joined = s
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = String::new();
    let mut newline_run = 0;
    for ch in joined.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run > 2 {
                continue;
            }
        } else {
            newline_run = 0;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

/// 把一条用户消息里所有 text 块中的 `@文件` 引用抬升成独立 file 块（排在正文之前）。开头附件
/// 引用从正文移除；中间引用则保留原文。只对真实用户消息（非 meta/系统注入）调用。
fn lift_file_refs(blocks: Vec<Block>, cwd: Option<&Path>) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len());
    for b in blocks {
        if b.kind == "text" {
            if let Some(t) = b.text.as_deref() {
                let (files, cleaned) = extract_file_refs(t, cwd);
                if !files.is_empty() {
                    out.extend(files);
                    if !cleaned.is_empty() {
                        out.push(text_block("text", &cleaned));
                    }
                    continue;
                }
            }
        }
        out.push(b);
    }
    out
}

/// Claude Code 把若干「系统注入」内容也写成 `type:"user"` 记录，但它们并不是用户
/// 手敲的 prose —— 前端不该渲染成「Me」气泡。这里按 JSONL 上的 flag（新版 CC）+
/// 内容标签（老版 CC 没有 flag）把它们归一成一个 meta_kind 字符串：
///   - `compact`         —— 上下文压缩摘要（`isCompactSummary`）
///   - `meta`            —— skill 注入 / 其它 `isMeta` 元信息
///   - `task-notification` —— 后台任务通知（`origin.kind` / `<task-notification>`）
///   - `system`          —— 其它系统来源（`promptSource == "system"`）
///   - `command-output`  —— slash / bash 命令的输出（`<local-command-stdout>` 等）
///   - `teammate-message` —— 多 agent 协作时对方会话发来的消息（`<teammate-message>`）
///
/// 返回 `None` 表示这是真正的用户消息。`blocks` 是已抽取好的块，用来嗅内容前缀。
/// 注意：调用点已先行丢弃 `[Image: source:]` 这类 isMeta 图片引用，不会进到这里。
fn classify_meta_kind(v: &Value, blocks: &[Block]) -> Option<String> {
    if v.get("isCompactSummary")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Some("compact".to_string());
    }
    if v.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
        return Some("meta".to_string());
    }
    // sourceToolUseID：工具执行后系统注入的 user 记录（技能正文 / 命令 slash 等）。
    // stream-json 事件可能不带 isMeta，但 sourceToolUseID 仍然存在。
    if v.get("sourceToolUseID").is_some_and(|x| x.is_string()) {
        return Some("meta".to_string());
    }
    let origin_kind = v
        .get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str);
    if origin_kind == Some("task-notification") {
        return Some("task-notification".to_string());
    }
    let prompt_source = v.get("promptSource").and_then(Value::as_str);
    // 真正的用户输入是 promptSource == "typed"（origin.kind == "human"）。其它 system
    // 来源（hook / 自动注入）都算系统消息。
    if prompt_source == Some("system") {
        return Some("system".to_string());
    }
    // 处理过程中到达的通知会被「排队」成 attachment（queued_command），
    // 用 attachment.commandMode 区分：用户手敲的是 "prompt"，系统通知是
    // "task-notification"。后者不是用户 prose。
    let cmd_mode = v
        .get("attachment")
        .and_then(|a| a.get("commandMode"))
        .and_then(Value::as_str);
    if cmd_mode == Some("task-notification") {
        return Some("task-notification".to_string());
    }
    // 内容标签兜底：老版本 CC 不写 promptSource/origin/commandMode，只能看正文前缀。
    // `<command-name>` / `<bash-input>` 是用户主动发起的命令调用，保持「Me」不动。
    let head = blocks
        .iter()
        .find(|b| b.kind == "text")
        .and_then(|b| b.text.as_deref())
        .unwrap_or("")
        .trim_start();
    if head.starts_with("<local-command-stdout>")
        || head.starts_with("<bash-stdout>")
        || head.starts_with("<bash-stderr>")
    {
        return Some("command-output".to_string());
    }
    if head.starts_with("<task-notification>") {
        return Some("task-notification".to_string());
    }
    // 上下文压缩摘要兜底：headless stream-json 的续聊摘要事件**不带** isCompactSummary
    // flag（只有落盘 transcript 才写），GUI chat 里只能靠 Claude Code 固定的续聊开场白
    // 识别，否则会被当成「Me」气泡渲染（还会把摘要里的 `@文件` 误抬成附件 chip）。
    if head.starts_with("This session is being continued from a previous conversation") {
        return Some("compact".to_string());
    }
    // 技能注入兜底：stream-json 的技能注入事件可能不带 isMeta flag，靠固定开场白识别。
    // 否则技能正文里的 `@anthropic-ai/sdk` 等会被 extract_file_refs 误抬成附件 chip。
    if head.starts_with("Base directory for this skill:") {
        return Some("meta".to_string());
    }
    // 多 agent 协作：对方会话发来的消息被注入成 user 记录（无 flag，只能看正文）。
    if head.starts_with("Another Claude session sent a message:")
        || head.contains("<teammate-message")
    {
        return Some("teammate-message".to_string());
    }
    None
}

/// 这条 `type:"user"` 记录是否是系统注入（而非用户手敲）。和 [`classify_meta_kind`]
/// 同源，但只看原始 `v`（不依赖已抽取的 blocks），给 `scan()` 选标题时过滤用。
/// 返回 true 的记录不该被当成「首条用户消息」拿去当会话标题。
fn is_injected_user(v: &Value) -> bool {
    if v.get("isCompactSummary")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    if v.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
        return true;
    }
    if v.get("sourceToolUseID").is_some_and(|x| x.is_string()) {
        return true;
    }
    if v.get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str)
        == Some("task-notification")
    {
        return true;
    }
    if v.get("promptSource").and_then(Value::as_str) == Some("system") {
        return true;
    }
    let head = user_text(v).unwrap_or_default();
    let head = head.trim_start();
    head.starts_with("<local-command-stdout>")
        || head.starts_with("<bash-stdout>")
        || head.starts_with("<bash-stderr>")
        || head.starts_with("<task-notification>")
        || head.starts_with("This session is being continued from a previous conversation")
        || head.starts_with("Base directory for this skill:")
        || head.starts_with("Another Claude session sent a message:")
        || head.contains("<teammate-message")
}

fn stringify_tool_result(c: Option<&Value>) -> String {
    match c {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => {
            let mut parts = Vec::new();
            for el in arr {
                match el.get("type").and_then(|x| x.as_str()) {
                    Some("text") => {
                        if let Some(s) = el.get("text").and_then(|x| x.as_str()) {
                            parts.push(s.to_string());
                        }
                    }
                    Some("image") => parts.push("[image]".to_string()),
                    _ => {}
                }
            }
            parts.join("\n")
        }
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// 把 Claude 的 structuredPatch 解析成带行号的 diff。
fn parse_structured_patch(v: &Value) -> Option<Vec<DiffHunk>> {
    let arr = v.as_array()?;
    if arr.is_empty() {
        return None;
    }
    let mut hunks = Vec::new();
    for h in arr {
        let old_start = h.get("oldStart").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let new_start = h.get("newStart").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let mut old_no = old_start;
        let mut new_no = new_start;
        let mut lines = Vec::new();
        if let Some(raw) = h.get("lines").and_then(|x| x.as_array()) {
            for l in raw {
                let s = l.as_str().unwrap_or("");
                let (kind, text): (&str, &str) = match s.chars().next() {
                    Some('+') => ("add", &s[1..]),
                    Some('-') => ("del", &s[1..]),
                    _ => ("ctx", s.strip_prefix(' ').unwrap_or(s)),
                };
                let (o, n) = match kind {
                    "add" => {
                        let n = new_no;
                        new_no += 1;
                        (None, Some(n))
                    }
                    "del" => {
                        let o = old_no;
                        old_no += 1;
                        (Some(o), None)
                    }
                    _ => {
                        let (o, n) = (old_no, new_no);
                        old_no += 1;
                        new_no += 1;
                        (Some(o), Some(n))
                    }
                };
                lines.push(DiffLine {
                    kind: kind.to_string(),
                    old_no: o,
                    new_no: n,
                    text: text.to_string(),
                });
            }
        }
        hunks.push(DiffHunk {
            old_start,
            new_start,
            lines,
        });
    }
    Some(hunks)
}

/// 把新标题镜像到 ~/.claude/sessions/<PID>.json 的 name 字段。
/// 这是 Claude Code 运行时维护的会话态文件，按 sessionId 找到匹配项，
/// 只改 name、保留其余字段。是 best-effort：找不到 / 解析失败 / 写失败都静默跳过，
/// 不影响 jsonl 里的持久标题。
fn mirror_runtime_name(session_id: &str, name: &str) {
    let dir = home().join(".claude").join("sessions");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = match fs::read_to_string(&p) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut v: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("sessionId").and_then(|x| x.as_str()) != Some(session_id) {
            continue;
        }
        if let Some(obj) = v.as_object_mut() {
            obj.insert("name".to_string(), Value::String(name.to_string()));
            if let Ok(serialized) = serde_json::to_string(&v) {
                let _ = fs::write(&p, serialized);
            }
        }
    }
}

/// 单遍扫描一个 jsonl，提取标题 / 时间 / 消息数等元信息。
/// Subagent JSONL 的路径形态：`.../<project_dir>/<parent_uuid>/subagents/agent-*.jsonl`。
/// 父目录名是 `subagents` 即认定它是子代理产物。
fn is_subagent_path(fp: &Path) -> bool {
    fp.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("subagents")
}

/// 把 `<projects>/<project_key>/<source_id>.jsonl` 克隆成一个全新、独立的会话文件。
///
/// 不走 `--fork-session`（它要等第一轮才落盘、列表里看不到），而是直接在磁盘上复制一份：
/// 每条记录用 `serde_json::Value` 原样保留所有字段，只重写 `sessionId` → 新 id、`uuid` →
/// 新 uuid，并据 uuid 映射重写 `parentUuid` / `leafUuid`，让克隆出来与原会话**零共享**
/// （Claude `--resume <newid>` 续聊时不会跨文件撞 uuid）。末尾追加一对 custom-title +
/// agent-name 写入 `title`，列表/详情即显示「<原名> fork」。返回新 session id。
fn fork_session(project_key: &str, source_id: &str, title: &str) -> Result<String, String> {
    let dir = projects_dir().join(project_key);
    let src = dir.join(format!("{source_id}.jsonl"));
    if !src.is_file() {
        return Err("源会话文件不存在，无法 fork".into());
    }
    let new_id = uuid::Uuid::new_v4().to_string();
    let content = fs::read_to_string(&src).map_err(|e| format!("读取源会话失败: {e}"))?;
    let out = fork_jsonl(&content, &new_id, title);
    let dst = dir.join(format!("{new_id}.jsonl"));
    fs::write(&dst, out).map_err(|e| format!("写入 fork 会话失败: {e}"))?;
    Ok(new_id)
}

/// 只克隆目标用户提问之前的 transcript。目标提问及它之后的部分回答、中断标记均不进入新会话。
fn fork_session_before_user_turn(
    project_key: &str,
    source_id: &str,
    title: &str,
    keep_user_turns: usize,
) -> Result<String, String> {
    let dir = projects_dir().join(project_key);
    let src = dir.join(format!("{source_id}.jsonl"));
    if !src.is_file() {
        return Err("源会话文件不存在，无法 fork".into());
    }
    let new_id = uuid::Uuid::new_v4().to_string();
    let content = fs::read_to_string(&src).map_err(|e| format!("读取源会话失败: {e}"))?;
    let out = fork_jsonl_before_user_turn(&content, &new_id, title, keep_user_turns)?;
    let dst = dir.join(format!("{new_id}.jsonl"));
    fs::write(&dst, out).map_err(|e| format!("写入 fork 会话失败: {e}"))?;
    Ok(new_id)
}

fn is_editable_user_prompt_record(v: &Value) -> bool {
    let Some(msg) = record_to_msg(v) else {
        return false;
    };
    if msg.role != "user" || msg.sidechain || msg.meta_kind.is_some() {
        return false;
    }
    if msg.blocks.iter().all(|block| {
        block.kind == "text"
            && block.text.as_deref().is_some_and(|text| {
                let text = text.trim();
                text.starts_with("<local-command-caveat>")
                    && text.ends_with("</local-command-caveat>")
            })
    }) {
        return false;
    }
    if msg.blocks.len() == 1 && msg.blocks[0].kind == "text" {
        let text = msg.blocks[0].text.as_deref().unwrap_or("").trim();
        if matches!(
            text,
            "[Request interrupted by user]" | "[Request interrupted by user for tool use]"
        ) || (text.starts_with("<system-reminder>")
            && text.ends_with("</system-reminder>")
            && text.contains("The user named this session"))
        {
            return false;
        }
    }
    msg.blocks.iter().any(|block| match block.kind.as_str() {
        "text" => block
            .text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty()),
        "image" => block
            .image_src
            .as_deref()
            .is_some_and(|src| src.starts_with("data:")),
        "file" => block
            .file_path
            .as_deref()
            .is_some_and(|path| !path.is_empty()),
        _ => false,
    })
}

fn fork_jsonl_before_user_turn(
    content: &str,
    new_id: &str,
    title: &str,
    keep_user_turns: usize,
) -> Result<String, String> {
    let mut prefix = String::new();
    let mut seen_user_turns = 0usize;
    let mut found_boundary = false;
    for line in content.lines() {
        let is_user_prompt = serde_json::from_str::<Value>(line)
            .ok()
            .is_some_and(|value| is_editable_user_prompt_record(&value));
        if is_user_prompt {
            if seen_user_turns == keep_user_turns {
                found_boundary = true;
                break;
            }
            seen_user_turns += 1;
        }
        if !line.trim().is_empty() {
            prefix.push_str(line);
            prefix.push('\n');
        }
    }
    if !found_boundary {
        return Err("源会话中找不到要编辑的用户提问".into());
    }
    Ok(fork_jsonl(&prefix, new_id, title))
}

/// 纯转换：把一份 transcript 重写成「克隆」版本 —— `sessionId` 全改成 `new_id`，每条记录的
/// `uuid` 换成全新 v4 并据映射重写 `parentUuid` / `leafUuid`（记录按时间序、父在子前 → 单遍
/// 即可），末尾追加 custom-title + agent-name 写入 `title`。其余字段原样保留。无副作用，便于单测。
fn fork_jsonl(content: &str, new_id: &str, title: &str) -> String {
    let mut id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = String::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(mut v) => {
                for key in ["parentUuid", "leafUuid"] {
                    if let Some(old) = v.get(key).and_then(|x| x.as_str()) {
                        if let Some(new) = id_map.get(old) {
                            v[key] = Value::String(new.clone());
                        }
                    }
                }
                if let Some(old) = v.get("uuid").and_then(|x| x.as_str()) {
                    let nu = uuid::Uuid::new_v4().to_string();
                    id_map.insert(old.to_string(), nu.clone());
                    v["uuid"] = Value::String(nu);
                }
                if v.get("sessionId").is_some() {
                    v["sessionId"] = Value::String(new_id.to_string());
                }
                out.push_str(&v.to_string());
                out.push('\n');
            }
            // 理论上不会有非 JSON 行；保守起见原样保留。
            Err(_) => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    // 标题：照搬 rename 的成对写法（custom-title + agent-name），让扫描出的展示名是 fork 名。
    let trimmed = validate_rename_name(title).unwrap_or("fork");
    for (t, field) in [("custom-title", "customTitle"), ("agent-name", "agentName")] {
        out.push_str(
            &serde_json::json!({ "type": t, field: trimmed, "sessionId": new_id }).to_string(),
        );
        out.push('\n');
    }
    out
}

/// 从一行 JSONL 的**前缀字节**里廉价提取顶层 `"field":"..."` 字符串值,不对整行做 serde 解析。
/// list 扫描只需要靠前的 type / cwd 等字段,而单行 message 正文可达上百 MB —— 对每行整体 serde
/// 解析是切到大会话项目时 16s 卡顿的根因。这里只在前缀里找,找不到（字段不在前缀 / 非字符串 /
/// 被截断）就返回 None,由调用方决定是否退回整行解析。转义按 JSON 规则用 serde 还原单个 token。
fn json_str_field_prefix(prefix: &[u8], field: &str) -> Option<String> {
    let mut needle = Vec::with_capacity(field.len() + 2);
    needle.push(b'"');
    needle.extend_from_slice(field.as_bytes());
    needle.push(b'"');
    let mut i = prefix
        .windows(needle.len())
        .position(|w| w == needle.as_slice())?
        + needle.len();
    while i < prefix.len() && (prefix[i] == b' ' || prefix[i] == b'\t') {
        i += 1;
    }
    if i >= prefix.len() || prefix[i] != b':' {
        return None;
    }
    i += 1;
    while i < prefix.len() && (prefix[i] == b' ' || prefix[i] == b'\t') {
        i += 1;
    }
    if i >= prefix.len() || prefix[i] != b'"' {
        return None;
    }
    let start = i; // 开引号
    i += 1;
    while i < prefix.len() {
        match prefix[i] {
            b'\\' => i += 2, // 跳过转义字符（可能越过前缀 → 视为截断）
            b'"' => {
                let raw = &prefix[start..=i]; // 含首尾引号
                return std::str::from_utf8(raw)
                    .ok()
                    .and_then(|s| serde_json::from_str::<String>(s).ok());
            }
            _ => i += 1,
        }
    }
    None // 前缀内未闭合
}

/// scan() 结果缓存：key = 绝对路径，value = (mtime, size, meta)。
/// 会话文件不变（mtime+size 一致）就直接返回克隆,避免每次切项目都把 500MB 会话重扫一遍。
/// 文件被追加/替换后 mtime 或 size 变 → key 不命中 → 重扫。是纯加速,不影响正确性。
type ScanCache = HashMap<PathBuf, (u64, u64, SessionMeta)>;
static SCAN_CACHE: OnceLock<Mutex<ScanCache>> = OnceLock::new();
fn scan_cache() -> &'static Mutex<ScanCache> {
    SCAN_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scan(fp: &Path) -> SessionMeta {
    let size = fs::metadata(fp).map(|m| m.len()).unwrap_or(0);
    let modified = mtime_millis(fp);
    if let Ok(cache) = scan_cache().lock() {
        if let Some((m, s, meta)) = cache.get(fp) {
            if *m == modified && *s == size {
                return meta.clone();
            }
        }
    }
    let meta = scan_uncached(fp, size, modified);
    if let Ok(mut cache) = scan_cache().lock() {
        cache.insert(fp.to_path_buf(), (modified, size, meta.clone()));
    }
    meta
}

fn scan_uncached(fp: &Path, size: u64, modified: u64) -> SessionMeta {
    let file_name = fp
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Subagent 文件的 session id 用父 session 的 UUID，让聚合器自然把它们的
    // cost / calls / tokens 合到父 session 下 —— 数据 0 丢失，session 计数不再被
    // inflated（典型场景：sidebar 显示 198 个 session，统计页之前算 298 个，差额
    // ~100 全是 subagent 文件被当成独立 session；现在两处一致）。
    let id = if is_subagent_path(fp) {
        fp.parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| file_name.trim_end_matches(".jsonl").to_string())
    } else {
        file_name.trim_end_matches(".jsonl").to_string()
    };

    // Claude Code `/rename <name>` 会成对追加 `custom-title` + `agent-name`
    // 两条记录（同值）。两者都识别，最后一条生效；否则回落到首条 user message。
    let mut first_user_title = String::new();
    let mut custom_title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut created: Option<String> = None;
    let mut message_count = 0usize;

    if let Ok(file) = fs::File::open(fp) {
        // 逐行读原始字节（read_until 复用同一个 buffer,避免为上百 MB 的行反复分配 String）；
        // 只对**必须**深挖的行（首条用户消息取标题、custom-title、attachment）做整行 serde 解析,
        // 其余（含巨大的 tool_result / image 行）只从前缀廉价取 type/cwd。见 json_str_field_prefix。
        let mut reader = BufReader::new(file);
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        // type/cwd 都在 message 正文之前,几百字节内；4KB 前缀足够覆盖且不触碰巨大正文。
        const PREFIX: usize = 4096;
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
            let line: &[u8] = {
                let mut s = &buf[..];
                while let [rest @ .., b'\n' | b'\r' | b' ' | b'\t'] = s {
                    s = rest;
                }
                while let [b'\n' | b'\r' | b' ' | b'\t', rest @ ..] = s {
                    s = rest;
                }
                s
            };
            if line.is_empty() {
                continue;
            }
            let prefix = &line[..line.len().min(PREFIX)];
            // cwd：原实现每行都取、最后一条生效（cd 后目录会变，见 scan_uses_last_cwd_after_cd）。
            if let Some(c) = json_str_field_prefix(prefix, "cwd") {
                cwd = Some(c);
            }
            let t = json_str_field_prefix(prefix, "type").unwrap_or_default();
            match t.as_str() {
                "custom-title" | "agent-name" => {
                    // 短记录,整行解析取标题字段。
                    if let Ok(v) = serde_json::from_slice::<Value>(line) {
                        let field = if t == "custom-title" {
                            "customTitle"
                        } else {
                            "agentName"
                        };
                        if let Some(ct) = v.get(field).and_then(|x| x.as_str()) {
                            let trimmed = ct.trim();
                            if !trimmed.is_empty() {
                                custom_title = Some(trimmed.to_string());
                            }
                        }
                    }
                }
                "user" | "assistant" => {
                    message_count += 1;
                    // created = 首条 user/assistant 的 timestamp；标题回落到首条「真正的」用户消息。
                    // timestamp 在正文之后（前缀里未必有）、user_text/is_injected 需要正文 —— 这两件
                    // 事都只在「还没拿到」时对该行整行解析,一旦拿齐后续行就只计数,不再深挖。
                    let need_created = created.is_none();
                    let need_title = first_user_title.is_empty() && t == "user";
                    if need_created || need_title {
                        if let Ok(v) = serde_json::from_slice::<Value>(line) {
                            if need_created {
                                created = v
                                    .get("timestamp")
                                    .and_then(|x| x.as_str())
                                    .map(|s| s.to_string());
                            }
                            // 跳过系统注入记录（skill 注入 / 压缩摘要 / 任务通知 / 命令输出），
                            // 否则 /dm-watch 这类会话的侧栏标题会变成注入正文。
                            if need_title && !is_injected_user(&v) {
                                if let Some(txt) = user_text(&v) {
                                    let cwd = v.get("cwd").and_then(|x| x.as_str()).map(Path::new);
                                    let (_, body) = extract_file_refs(&txt, cwd);
                                    let clean = clean_title(&body);
                                    if !clean.is_empty() {
                                        first_user_title = clean;
                                    }
                                }
                            }
                        }
                    }
                }
                "attachment" => {
                    // 排队输入的消息（attachment/queued_command）也算一条用户消息。
                    if let Ok(v) = serde_json::from_slice::<Value>(line) {
                        if queued_command_blocks(&v).is_some() {
                            message_count += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let title = custom_title.unwrap_or_else(|| {
        if first_user_title.is_empty() {
            "(untitled session)".to_string()
        } else {
            first_user_title
        }
    });
    SessionMeta {
        id,
        file_name,
        path: fp.to_string_lossy().to_string(),
        title,
        cwd,
        created,
        modified,
        size,
        message_count,
        codex_app_list_rank: None,
        codex_app_list_scanned: 0,
        codex_app_first_page_size: 50,
        codex_app_first_page_position: 0,
        codex_internal: false,
        codex_archived: false,
    }
}

fn read(path: &str) -> Result<Vec<Msg>, String> {
    let file = fs::File::open(path).map_err(|e| format!("Failed to open session: {e}"))?;
    let mut msgs = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(msg) = record_to_msg(&v) {
            msgs.push(msg);
        }
    }
    // Claude Code 把 `/compact` 的「上下文摘要」记录写在续聊上下文的**开头** —— 文件顺序上
    // 它排在触发它的 `/compact` 命令之前，但它的 timestamp 其实晚得多（压缩完成的时刻）。
    // 直接按文件顺序渲染会让「摘要结果」跑到用户的 `/compact` 消息上方（很反直觉）。
    // 仅当存在压缩摘要、且每条消息都带 timestamp 时，按 timestamp **稳定**排序把摘要归位 ——
    // 其余消息本就按时间写入，稳定排序不会扰动它们（ISO-8601 + `Z`，字典序即时间序）。
    if msgs
        .iter()
        .any(|m| m.meta_kind.as_deref() == Some("compact"))
        && msgs.iter().all(|m| m.timestamp.is_some())
    {
        msgs.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    }
    Ok(msgs)
}

/// 把单条 JSONL 记录（或 stream-json 事件里同形的 `message` 记录）解析成一条 `Msg`。
/// 返回 `None` 表示这条记录不产生气泡：非 user/assistant/queued attachment、空内容、
/// 或 `isMeta` 的 `[Image: source:]` 引用副本。
///
/// 既给 [`read`] 逐行复用，也给 stream-json 的 [`parse_chat_line`] 复用 —— stream-json 的
/// assistant/user 事件的 `message` 字段与 JSONL 记录同形，所以记录→`Block` 的归一逻辑
/// 只此一份，GUI live chat 与离线回看走完全一致的渲染。
pub(crate) fn record_to_msg(v: &Value) -> Option<Msg> {
    let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    // 用户在 Claude 处理中排队输入的消息不是常规 user 记录，而是
    // `attachment`（attachment.type == "queued_command"）。常规解析只认
    // user/assistant，会整条丢掉它 —— 这里单独补成一条 user 气泡。
    if t == "attachment" {
        let blocks = queued_command_blocks(v)?;
        // 排队进来的可能是用户手敲消息（→ Me），也可能是处理中到达的
        // 任务通知（commandMode == "task-notification" → 系统块）。
        let meta_kind = classify_meta_kind(v, &blocks);
        return Some(Msg {
            uuid: v
                .get("uuid")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            role: "user".to_string(),
            timestamp: v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            model: None,
            sidechain: v
                .get("isSidechain")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            blocks,
            meta_kind,
        });
    }
    // `/context`、`/model` 等本地 slash 命令：on-disk transcript 把它们的「调用回显」
    // （<command-name>…）和「输出」（<local-command-stdout>…）都落成 `system` /
    // `subtype:"local_command"` 记录。read（刷新/离线回看）原本只认 user/assistant，会整条
    // 丢掉它们 —— 这正是刷新后 /context 卡片消失、命令输出全无的原因。这里把它当作等价的
    // user 记录交给既有逻辑：classify_meta_kind 会把 <command-name> 归为普通命令气泡、把
    // <local-command-stdout> 归为 command-output 折叠块（与磁盘里以 user 记录落盘的 /effort
    // 等命令完全一致）。/context 的 command-output 块再由前端 contextUsageOf 升级成可视化卡片。
    // **不能**在这里造 model=<synthetic> 的 assistant 记录 —— 那会把底栏模型选择器带歪。
    if t == "system" && v.get("subtype").and_then(|x| x.as_str()) == Some("local_command") {
        let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
        if content.trim().is_empty() {
            return None;
        }
        let blocks = vec![text_block("text", content)];
        let meta_kind = classify_meta_kind(v, &blocks);
        return Some(Msg {
            uuid: v
                .get("uuid")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            role: "user".to_string(),
            timestamp: v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            model: None,
            sidechain: v
                .get("isSidechain")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            blocks,
            meta_kind,
        });
    }
    if t != "user" && t != "assistant" {
        return None;
    }
    let sidechain = v
        .get("isSidechain")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let uuid = v
        .get("uuid")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let timestamp = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let message = v.get("message");
    let model = message
        .and_then(|m| m.get("model"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    let mut blocks = Vec::new();
    if let Some(content) = message.and_then(|m| m.get("content")) {
        match content {
            Value::String(s) if !s.trim().is_empty() => {
                blocks.push(text_block("text", s));
            }
            Value::Array(arr) => {
                for el in arr {
                    let et = el.get("type").and_then(|x| x.as_str()).unwrap_or("");
                    match et {
                        "text" => {
                            if let Some(s) = el.get("text").and_then(|x| x.as_str()) {
                                if !s.trim().is_empty() {
                                    blocks.push(text_block("text", s));
                                }
                            }
                        }
                        "thinking" => {
                            if let Some(s) = el.get("thinking").and_then(|x| x.as_str()) {
                                if !s.trim().is_empty() {
                                    blocks.push(text_block("thinking", s));
                                }
                            }
                        }
                        "tool_use" => {
                            let name = el
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("tool")
                                .to_string();
                            let input = el
                                .get("input")
                                .map(|i| serde_json::to_string_pretty(i).unwrap_or_default());
                            let id = el.get("id").and_then(|x| x.as_str()).map(|s| s.to_string());
                            blocks.push(Block {
                                kind: "tool_use".to_string(),
                                tool_name: Some(name),
                                tool_input: input,
                                tool_id: id,
                                ..Default::default()
                            });
                        }
                        "tool_result" => {
                            let id = el
                                .get("tool_use_id")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string());
                            let is_error = el
                                .get("is_error")
                                .and_then(|x| x.as_bool())
                                .unwrap_or(false);
                            let txt = stringify_tool_result(el.get("content"));
                            // 同一条记录顶层的 toolUseResult 携带文件改动的结构化 diff。
                            // stream-json 事件没有这个顶层字段 → diff/file_path 为 None，
                            // 工具结果以纯文本呈现，离线回看时再带出结构化 diff。
                            let tur = v.get("toolUseResult");
                            let file_path = tur
                                .and_then(|t| t.get("filePath"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string());
                            let diff = tur
                                .and_then(|t| t.get("structuredPatch"))
                                .and_then(parse_structured_patch);
                            blocks.push(Block {
                                kind: "tool_result".to_string(),
                                text: Some(txt),
                                tool_id: id,
                                is_error,
                                file_path,
                                diff,
                                ..Default::default()
                            });
                        }
                        "image" => {
                            if let Some(src) = image_src(el) {
                                blocks.push(Block {
                                    kind: "image".to_string(),
                                    image_src: Some(src),
                                    ..Default::default()
                                });
                            } else {
                                blocks.push(text_block("text", "[image]"));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if blocks.is_empty() {
        return None;
    }
    // 本地 slash 命令（/context 等）在落盘 transcript 里跟着一条 model=<synthetic> 的
    // 「No response requested.」assistant 占位 —— live stream 根本不吐它。离线回看也别渲染，
    // 否则刚还原出来的 /context 卡片旁边会多一个空洞的占位气泡，跟 live 观感对不上。
    if t == "assistant"
        && model.as_deref() == Some("<synthetic>")
        && blocks.len() == 1
        && blocks[0].kind == "text"
        && blocks[0].text.as_deref().map(str::trim) == Some("No response requested.")
    {
        return None;
    }
    // Claude 把用户贴图拆成两条 user 记录：一条是带 base64 的真实消息，
    // 紧跟一条 `isMeta:true` 的 `[Image: source: <local-path>]` 引用。
    // 已经在上一条里渲染过真实图，跳过 meta 那条避免出现重复气泡。
    if t == "user" && is_image_source_meta(v, &blocks) {
        return None;
    }
    // 系统注入的 user 记录（压缩摘要 / skill / 任务通知 / 命令输出）归类，
    // 让前端把它们渲染成低调的系统块而非「Me」气泡。assistant 永远是 None。
    let meta_kind = if t == "user" {
        classify_meta_kind(v, &blocks)
    } else {
        None
    };
    // 真实用户消息：把正文里的 `@文件` 引用抬升成 file 块（点击外部打开），系统/meta
    // 注入的伪 user 记录不动（它们的 `@...` 多是说明文字，不该当成附件）。
    if t == "user" && meta_kind.is_none() {
        let cwd = v.get("cwd").and_then(|x| x.as_str()).map(Path::new);
        blocks = lift_file_refs(blocks, cwd);
    }
    Some(Msg {
        uuid,
        role: t.to_string(),
        timestamp,
        model,
        sidechain,
        blocks,
        meta_kind,
    })
}

// ============================ §10.1 slash 指令磁盘发现 ============================

/// 扫出 GUI chat `/` 浮层可用项 —— **命令 + 技能**，按来源收集后分组排序。
///   命令：项目 `<cwd>/.claude/commands` > 用户 `~/.claude/commands` > 已启用插件 `<install>/commands`
///   技能：项目 `<cwd>/.claude/skills`   > 用户 `~/.claude/skills`   > 已启用插件 `<install>/skills`
/// 命令、技能各自按名字去重（先到先得 = 项目 > 用户 > 插件）。**不含 TUI 内置指令**（headless
/// 不展开）。排序：命令在前、技能在后，组内按展示名不分大小写。
pub(crate) fn chat_slash_commands(cwd: &str) -> Vec<crate::types::SlashCommand> {
    use crate::types::SlashCommand;
    let mut out: Vec<SlashCommand> = Vec::new();
    let mut seen_cmd: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_skill: std::collections::HashSet<String> = std::collections::HashSet::new();

    let proj_name = project_basename(cwd);
    let plugins = enabled_plugins(cwd); // (插件名, 安装目录)，仅当前 cwd 下已启用

    // ---- 命令 ----
    // 项目 / 用户命令无命名空间（namespace=None）；插件命令必须带 `<plugin>:` 前缀（如 `/codex:review`）
    // 才是 CLI 认的真实调用名 —— 用插件名作 namespace，展示角标仍用美化名。
    let proj_cmds = Path::new(cwd).join(".claude").join("commands");
    scan_commands_dir(
        &proj_cmds,
        &proj_cmds,
        "project",
        proj_name.as_deref(),
        None,
        &mut out,
        &mut seen_cmd,
    );
    let user_cmds = home().join(".claude").join("commands");
    scan_commands_dir(
        &user_cmds,
        &user_cmds,
        "user",
        None,
        None,
        &mut out,
        &mut seen_cmd,
    );
    for (plugin, install) in &plugins {
        let dir = install.join("commands");
        let badge = prettify_name(plugin);
        scan_commands_dir(
            &dir,
            &dir,
            "plugin",
            Some(&badge),
            Some(plugin),
            &mut out,
            &mut seen_cmd,
        );
    }

    // ---- 技能 ----
    let proj_skills = Path::new(cwd).join(".claude").join("skills");
    scan_skills_dir(
        &proj_skills,
        "project",
        proj_name.as_deref(),
        None,
        &mut out,
        &mut seen_skill,
    );
    scan_skills_dir(
        &Path::new(cwd).join(".agents").join("skills"),
        "project",
        proj_name.as_deref(),
        None,
        &mut out,
        &mut seen_skill,
    );
    scan_skills_dir(
        &home().join(".claude").join("skills"),
        "user",
        None,
        None,
        &mut out,
        &mut seen_skill,
    );
    scan_skills_dir(
        &home().join(".agents").join("skills"),
        "user",
        None,
        None,
        &mut out,
        &mut seen_skill,
    );
    for (plugin, install) in &plugins {
        let badge = prettify_name(plugin);
        scan_skills_dir(
            &install.join("skills"),
            "plugin",
            Some(&badge),
            Some(plugin),
            &mut out,
            &mut seen_skill,
        );
    }

    // 排序：先按 kind（命令在前、技能在后），同 kind 内**按来源聚拢**（项目 > 用户 > 插件，
    // 插件再按插件名归堆），最后组内按展示名不分大小写 —— 同一来源的项排在一起，不再交错。
    out.sort_by(|a, b| {
        kind_rank(&a.kind)
            .cmp(&kind_rank(&b.kind))
            .then_with(|| origin_rank(&a.origin).cmp(&origin_rank(&b.origin)))
            .then_with(|| a.origin_name.cmp(&b.origin_name))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

/// 分组排序权重：命令在前(0)，技能在后(1)。
fn kind_rank(kind: &str) -> u8 {
    if kind == "command" {
        0
    } else {
        1
    }
}

/// 同 kind 内的来源聚拢顺序：项目(0) > 用户(1) > 插件(2)（与去重优先级一致）。
fn origin_rank(origin: &str) -> u8 {
    match origin {
        "project" => 0,
        "user" => 1,
        _ => 2,
    }
}

/// cwd 末段作为项目展示名（来源角标用）。
pub(crate) fn project_basename(cwd: &str) -> Option<String> {
    Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
}

/// kebab / snake / 空格分词后每段首字母大写：`animejs`→`Animejs`、
/// `create-promo-video`→`Create Promo Video`。
fn prettify_name(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 读 `~/.claude/plugins/installed_plugins.json` + `settings.json` 的 `enabledPlugins`，返回
/// 当前 cwd 下「已启用」插件的 (插件名, 安装目录)。判定一个插件已启用且可用：
///   · 用户全局启用（settings.enabledPlugins[key]==true）→ 取其 user 作用域安装记录；或
///   · 某条安装记录 projectPath==cwd（项目 / local 作用域）→ 该项目下自动启用。
/// 取不到安装路径的跳过。**插件名 = key 里 `@` 前一段**（= plugin.json 的 name），命令/技能调用
/// 要靠它加 `<plugin>:` 命名空间前缀（如 `/codex:review`）—— 故这里返回**原始名**而非美化名，
/// 美化只在显示角标时做。
fn enabled_plugins(cwd: &str) -> Vec<(String, PathBuf)> {
    let claude = home().join(".claude");
    let installed: Value =
        fs::read_to_string(claude.join("plugins").join("installed_plugins.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
    let settings: Value = fs::read_to_string(claude.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let enabled_map = settings.get("enabledPlugins").and_then(|v| v.as_object());
    let Some(map) = installed.get("plugins").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (key, records) in map {
        let Some(recs) = records.as_array() else {
            continue;
        };
        let globally_enabled = enabled_map
            .and_then(|m| m.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let install = if globally_enabled {
            recs.iter()
                .find(|r| r.get("scope").and_then(Value::as_str) == Some("user"))
                .or_else(|| recs.first())
                .and_then(|r| r.get("installPath").and_then(Value::as_str))
        } else {
            recs.iter()
                .find(|r| r.get("projectPath").and_then(Value::as_str) == Some(cwd))
                .and_then(|r| r.get("installPath").and_then(Value::as_str))
        };
        let Some(install) = install else { continue };
        let name = key.split('@').next().unwrap_or(key);
        out.push((name.to_string(), PathBuf::from(install)));
    }
    out
}

/// 递归扫 `commands` 下的 `*.md`（项目 / 用户 / 插件同一套）。基础名 = 相对 `root` 的路径去
/// 扩展名、`/`→`:`（如 `git/commit.md` → `git:commit`），对齐 Claude 命令命名。
/// `namespace=Some("codex")`（插件）时再前置 `codex:` → `/codex:review`，这才是 CLI 认的真实调用名。
fn scan_commands_dir(
    dir: &Path,
    root: &Path,
    origin: &str,
    origin_name: Option<&str>,
    namespace: Option<&str>,
    out: &mut Vec<crate::types::SlashCommand>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_commands_dir(&path, root, origin, origin_name, namespace, out, seen);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let base = rel
            .with_extension("")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, ":");
        if base.is_empty() {
            continue;
        }
        let name = match namespace {
            Some(ns) => format!("{ns}:{base}"),
            None => base,
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let description = md_description(&path).unwrap_or_default();
        let argument_hint = read_frontmatter(&path)
            .get("argument-hint")
            .filter(|s| !s.is_empty())
            .map(|s| cap_desc(s));
        out.push(crate::types::SlashCommand {
            title: format!("/{name}"),
            name,
            description,
            kind: "command".to_string(),
            origin: origin.to_string(),
            origin_name: origin_name.map(str::to_string),
            argument_hint,
        });
    }
}

/// 扫 `skills/*/SKILL.md`（项目 / 用户 / 插件），收**全部**技能（不再过滤 `user-invocable`）。
/// 基础名取 frontmatter `name`、回退目录名；`namespace=Some("codex")`（插件）时调用名前置
/// `codex:` → `/codex:obsidian-cli`（与命令一致，对齐 Claude 的 `plugin:skill` 命名）。
/// 展示名（title）美化**基础名**，描述优先 frontmatter `description`。
pub(crate) fn scan_skills_dir(
    skills_dir: &Path,
    origin: &str,
    origin_name: Option<&str>,
    namespace: Option<&str>,
    out: &mut Vec<crate::types::SlashCommand>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(entries) = fs::read_dir(skills_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let fm = read_frontmatter(&skill_md);
        let base = fm
            .get("name")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
        if base.is_empty() {
            continue;
        }
        let title = prettify_name(&base);
        let name = match namespace {
            Some(ns) => format!("{ns}:{base}"),
            None => base,
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let description = fm
            .get("description")
            .filter(|s| !s.is_empty())
            .map(|d| cap_desc(d))
            .or_else(|| md_description(&skill_md))
            .unwrap_or_default();
        out.push(crate::types::SlashCommand {
            title,
            name,
            description,
            kind: "skill".to_string(),
            origin: origin.to_string(),
            origin_name: origin_name.map(str::to_string),
            argument_hint: None,
        });
    }
}

/// 取命令 `.md` 的描述：优先 frontmatter `description:`，否则正文首个非空非标题行。封顶 200 字符。
fn md_description(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let fm = parse_frontmatter(&content);
    if let Some(d) = fm.get("description") {
        if !d.is_empty() {
            return Some(cap_desc(d));
        }
    }
    // 跳过 frontmatter 块后找正文首行。
    let body = strip_frontmatter(&content);
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        return Some(cap_desc(line));
    }
    None
}

fn cap_desc(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 200 {
        format!("{}…", s.chars().take(200).collect::<String>())
    } else {
        s.to_string()
    }
}

/// 读并解析一个文件的 YAML frontmatter（极简：`key: value` 行，value 去引号）。无则空 map。
fn read_frontmatter(path: &Path) -> std::collections::HashMap<String, String> {
    fs::read_to_string(path)
        .map(|c| parse_frontmatter(&c))
        .unwrap_or_default()
}

/// 解析 `---\n...\n---` 之间的简单 `key: value`。仅取标量行，忽略嵌套 / 列表。
fn parse_frontmatter(content: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let trimmed = content.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return map;
    }
    let mut lines = trimmed.lines().peekable();
    lines.next(); // 开头的 ---
    while let Some(line) = lines.next() {
        if line.trim() == "---" {
            break;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        if key.is_empty() || key.starts_with('#') {
            continue;
        }
        let vt = v.trim();
        // YAML 块标量 `key: |` / `key: >`（含 |- >- |+ 等）：值是后续更深缩进的行，
        // 折叠空白后并成一行（description 常这么写多行，否则会取到字面 "|"）。
        if vt.starts_with('|') || vt.starts_with('>') {
            let mut buf: Vec<String> = Vec::new();
            while let Some(next) = lines.peek() {
                if next.trim() == "---" {
                    break;
                }
                if next.is_empty() || next.starts_with([' ', '\t']) {
                    buf.push(lines.next().unwrap().trim().to_string());
                } else {
                    break; // 顶格的下一个键 → 块结束
                }
            }
            map.insert(
                key.to_string(),
                buf.join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        } else {
            let val = vt.trim_matches(['"', '\'']).trim();
            map.insert(key.to_string(), val.to_string());
        }
    }
    map
}

/// 去掉开头的 frontmatter 块，返回正文（找闭合的 `---` 行之后）。无 frontmatter 原样返回。
fn strip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return content;
    }
    let Some(first_nl) = trimmed.find('\n') else {
        return "";
    };
    let after_open = &trimmed[first_nl + 1..];
    let mut pos = 0;
    for line in after_open.split_inclusive('\n') {
        if line.trim() == "---" {
            return &after_open[pos + line.len()..];
        }
        pos += line.len();
    }
    content
}

/// 把 stream-json 子进程 stdout 的一行翻成统一的 [`ChatEvent`]。Claude 事件形状：
///   `{"type":"system","subtype":"init","session_id":"...",...}`  → `Init`
///   `{"type":"assistant","message":{...},"session_id":"..."}`     → `Message`（复用 `record_to_msg`）
///   `{"type":"user","message":{"content":[tool_result…]},...}`    → `Message`
///   `{"type":"result","subtype":"success","usage":{...},...}`     → `Result`（一轮结束）
/// 其它（`stream_event` 局部增量 / 未知类型）→ `Ignore`。
pub(crate) fn parse_chat_line(line: &str) -> ChatEvent {
    let line = line.trim();
    if line.is_empty() {
        return ChatEvent::Ignore;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return ChatEvent::Ignore;
    };
    match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
        "assistant" | "user" => match record_to_msg(&v) {
            Some(msg) => ChatEvent::Message(msg),
            None => ChatEvent::Ignore,
        },
        "system" if v.get("subtype").and_then(|x| x.as_str()) == Some("init") => {
            ChatEvent::Init {
                session_id: v
                    .get("session_id")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                // init 携带 apiKeySource：用来区分订阅登录（"none"）与 API key 计费。
                api_key_source: v
                    .get("apiKeySource")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
            }
        }
        "result" => {
            let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let ok = !is_error && v.get("subtype").and_then(|x| x.as_str()) == Some("success");
            let usage = v.get("usage").map(parse_stream_usage);
            ChatEvent::Result { ok, usage }
        }
        // token 级流式：`stream_event` 包裹标准 Anthropic SSE，payload 在 `.event`。
        "stream_event" => parse_stream_event(v.get("event")),
        // 交互式工具审批：`--permission-prompt-tool stdio` 把被门控工具的请求以控制协议
        // 发来（`{"type":"control_request","request_id":...,"request":{"subtype":"can_use_tool",...}}`）。
        // 只认 `can_use_tool`（无 MCP/hook 时这是唯一会出现的 control_request）；其余忽略。
        "control_request" => parse_can_use_tool(&v),
        // 限额事件不走流解析了：额度改由 OAuth 用量接口（usage_api）随时全量拉取。
        _ => ChatEvent::Ignore,
    }
}

/// 把一条 `control_request`/`can_use_tool` 归一成 [`ChatEvent::Permission`]。非该子类型
/// （或缺 `request_id` / `tool_name`）→ `Ignore`。`input` / `permission_suggestions` 原样
/// 透传给前端（回写时 `updatedInput` / `updatedPermissions` 用得到）。
fn parse_can_use_tool(v: &Value) -> ChatEvent {
    let req = v.get("request");
    if req.and_then(|r| r.get("subtype")).and_then(Value::as_str) != Some("can_use_tool") {
        return ChatEvent::Ignore;
    }
    let (Some(request_id), Some(req)) = (v.get("request_id").and_then(Value::as_str), req) else {
        return ChatEvent::Ignore;
    };
    let Some(tool_name) = req.get("tool_name").and_then(Value::as_str) else {
        return ChatEvent::Ignore;
    };
    // AskUserQuestion 同走 can_use_tool，但语义是「向用户提问」而非「门控工具」——
    // 拆成独立事件，前端弹选择题卡片而非权限对话框。`questions` 原样透传（回写 decision
    // 的 updatedInput 要带回它）；缺/非数组也照发，前端渲染空卡 + 取消按钮以免 CLI 卡死。
    if tool_name == "AskUserQuestion" {
        return ChatEvent::Question(crate::types::ChatQuestionRequest {
            request_id: request_id.to_string(),
            questions: req
                .get("input")
                .and_then(|i| i.get("questions"))
                .cloned()
                .unwrap_or(Value::Null),
            keep_after_turn: false,
        });
    }
    ChatEvent::Permission(crate::types::ChatPermissionRequest {
        request_id: request_id.to_string(),
        tool_name: tool_name.to_string(),
        input: req.get("input").cloned().unwrap_or(Value::Null),
        description: req
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        permission_suggestions: req.get("permission_suggestions").cloned(),
    })
}

/// 把一个 `stream_event.event`（标准 Anthropic 流式事件）归一成 `ChatEvent::Delta`。
/// 只关心块生命周期 + 文本增量；`message_start/delta/stop`、`signature_delta`、
/// `input_json_delta` 对 MVP 打字机无用 → `Ignore`（权威 `assistant` 记录会定稿）。
fn parse_stream_event(event: Option<&Value>) -> ChatEvent {
    let Some(ev) = event else {
        return ChatEvent::Ignore;
    };
    let index = ev.get("index").and_then(Value::as_u64).unwrap_or(0);
    match ev.get("type").and_then(Value::as_str).unwrap_or("") {
        "content_block_start" => ChatEvent::Delta(ChatDelta {
            index,
            phase: "start".to_string(),
            kind: ev
                .get("content_block")
                .and_then(|c| c.get("type"))
                .and_then(Value::as_str)
                .map(str::to_string),
            text: None,
        }),
        "content_block_delta" => {
            let delta = ev.get("delta");
            let dtype = delta
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            // text_delta → .text；thinking_delta → .thinking。其它（signature/input_json）忽略。
            let (kind, text) = match dtype {
                "text_delta" => (
                    "text",
                    delta.and_then(|d| d.get("text")).and_then(Value::as_str),
                ),
                "thinking_delta" => (
                    "thinking",
                    delta
                        .and_then(|d| d.get("thinking"))
                        .and_then(Value::as_str),
                ),
                _ => ("", None),
            };
            match text {
                Some(t) => ChatEvent::Delta(ChatDelta {
                    index,
                    phase: "delta".to_string(),
                    kind: Some(kind.to_string()),
                    text: Some(t.to_string()),
                }),
                None => ChatEvent::Ignore,
            }
        }
        "content_block_stop" => ChatEvent::Delta(ChatDelta {
            index,
            phase: "stop".to_string(),
            kind: None,
            text: None,
        }),
        _ => ChatEvent::Ignore,
    }
}

/// 从 `result` 事件的 `usage` 对象抽出 `UsageSummary`（字段同 assistant.message.usage）。
fn parse_stream_usage(u: &Value) -> UsageSummary {
    UsageSummary {
        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_creation_input_tokens: u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_input_tokens: u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        ..Default::default()
    }
    .finalize()
}

// ---- read_turns（统计聚合用）---------------------------------------------
//
// 单遍走 JSONL 把每条消息抽成结构化的 Turn / CallRecord。和 `read()` 的区别：
//   - 不返回 UI 用的 Block 结构（thinking / text / image / tool_result 全跳）
//   - 在每个 assistant message 上把 usage / model 顺便挖出来
//   - tool_use 块只关心 name 和 input —— name 直接进 tools，Bash 的 input 抽
//     第一个命令词进 bash_commands；mcp__server__tool 前缀抽 server 进 mcp_servers
//
// 一条 user 消息开启一个 Turn；之后的 assistant 消息持续 push 进该 Turn 的 calls。
// 没有 user 消息打头的孤儿 assistant（很少见但合法）合并到上一个 Turn 末尾。
fn read_turns(fp: &Path) -> Vec<Turn> {
    let session_id = fp
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.trim_end_matches(".jsonl").to_string())
        .unwrap_or_default();
    let file = match fs::File::open(fp) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut turns: Vec<Turn> = Vec::new();
    let mut cur: Option<Turn> = None;
    let mut project_path: String = String::new();

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
            project_path = c.to_string();
        }
        let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if t != "user" && t != "assistant" {
            continue;
        }
        let ts_ms = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso8601_ms)
            .unwrap_or(0);

        if t == "user" {
            // 把上一轮（含 calls 的）写出
            if let Some(prev) = cur.take() {
                turns.push(prev);
            }
            let user_text = user_text(&v).unwrap_or_default();
            cur = Some(Turn {
                user_message: user_text,
                project_path: project_path.clone(),
                session_id: session_id.clone(),
                calls: Vec::new(),
                timestamp_ms: ts_ms,
            });
            continue;
        }

        // assistant
        let message = match v.get("message") {
            Some(m) => m,
            None => continue,
        };
        let model = message
            .get("model")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        // Claude `message.id`（"msg_xxx"）—— 用于跨文件去重。fork / continue / sub-agent
        // JSONL 之间常见同一条 assistant 消息被多个文件抄录，按这个 id 跳过避免翻倍。
        let message_id = message
            .get("id")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        // usage：claude 是 message.usage.{input_tokens, output_tokens, cache_*}
        let mut usage = UsageSummary::default();
        if let Some(u) = message.get("usage") {
            usage.input_tokens = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
            usage.output_tokens = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
            // cache_creation 有两种形状：
            //   legacy: cache_creation_input_tokens = 整数（不分 tier）
            //   split:  cache_creation = { ephemeral_5m_input_tokens: N, ephemeral_1h_input_tokens: M }
            // 两者通常同时出现，legacy 字段 = 5m + 1h。我们这里把 total 收齐到
            // `cache_creation_input_tokens`，再把 1h 子集单独记到 `_1h_` 字段供 cost 算 2× 计费。
            let legacy = u
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let cc = u.get("cache_creation");
            let fivem = cc
                .and_then(|x| x.get("ephemeral_5m_input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let one_h = cc
                .and_then(|x| x.get("ephemeral_1h_input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            // 缺哪个用哪个：拼一份 5m + 1h；如果 split 是 0 / 缺失，退回 legacy。
            let split_total = fivem.saturating_add(one_h);
            usage.cache_creation_input_tokens = legacy.max(split_total);
            // 1h 子集要 ≤ total，钳一下防御性。Anthropic 偶尔分裂上报、legacy 缺一拍。
            usage.cache_creation_1h_input_tokens = one_h.min(usage.cache_creation_input_tokens);
            usage.cache_read_input_tokens = u
                .get("cache_read_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            usage = usage.finalize();
        }
        // 工具集合
        let mut tools: Vec<String> = Vec::new();
        let mut bash_commands: Vec<String> = Vec::new();
        let mut mcp_servers: Vec<String> = Vec::new();
        let mut has_agent_spawn = false;
        if let Some(content) = message.get("content").and_then(|x| x.as_array()) {
            for el in content {
                if el.get("type").and_then(|x| x.as_str()) != Some("tool_use") {
                    continue;
                }
                let name = el
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                if matches!(name.as_str(), "Task" | "Agent" | "task_spawn") {
                    has_agent_spawn = true;
                }
                if name == "Bash" || name == "BashTool" {
                    if let Some(input) = el.get("input") {
                        // input 可能是 object 或 string；shell_util 接受字符串
                        let raw = match input {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        if let Some(cmd) = shell_util::extract_first_command(&raw) {
                            bash_commands.push(cmd);
                        }
                    }
                }
                if let Some(server) = shell_util::extract_mcp_server(&name) {
                    mcp_servers.push(server);
                }
                tools.push(name);
            }
        }
        let cost = if usage.total == 0 {
            0.0
        } else {
            pricing::cost_usd(&model, &usage)
        };
        let call = CallRecord {
            model,
            message_id,
            usage,
            cost_usd: cost,
            tools,
            bash_commands,
            mcp_servers,
            has_plan_mode: false, // Claude 不显式记 plan mode；用 ExitPlanMode 工具名兜底判断
            has_agent_spawn,
        };
        if let Some(turn) = cur.as_mut() {
            let mut call = if call.tools.iter().any(|t| t == "ExitPlanMode") {
                CallRecord {
                    has_plan_mode: true,
                    ..call
                }
            } else {
                call
            };
            // Streaming: Claude Code writes multiple assistant lines per API call
            // with the same message.id — intermediates carry 0 usage, only the final
            // entry has the real token counts. Coalesce by replacing the earlier
            // (0-usage) entry so the aggregator's cross-file dedup keeps the real one.
            if let Some(ref id) = call.message_id {
                if let Some(existing) = turn
                    .calls
                    .iter_mut()
                    .find(|c| c.message_id.as_deref() == Some(id))
                {
                    // Merge: keep whichever side has more data.
                    if call.usage.total >= existing.usage.total {
                        existing.usage = call.usage;
                        existing.cost_usd = call.cost_usd;
                    }
                    if !call.model.is_empty() && call.model != "<synthetic>" {
                        existing.model = call.model;
                    }
                    if !call.tools.is_empty() {
                        existing.tools.append(&mut call.tools);
                    }
                    if !call.bash_commands.is_empty() {
                        existing.bash_commands.append(&mut call.bash_commands);
                    }
                    if !call.mcp_servers.is_empty() {
                        existing.mcp_servers.append(&mut call.mcp_servers);
                    }
                    existing.has_plan_mode |= call.has_plan_mode;
                    existing.has_agent_spawn |= call.has_agent_spawn;
                    continue;
                }
            }
            turn.calls.push(call);
        } else {
            // 孤儿 assistant（合法但少见）：起一个空 user_message 的占位 turn
            cur = Some(Turn {
                user_message: String::new(),
                project_path: project_path.clone(),
                session_id: session_id.clone(),
                calls: vec![call],
                timestamp_ms: ts_ms,
            });
        }
    }
    if let Some(t) = cur {
        turns.push(t);
    }

    // Third-party models (e.g. mimo-v2.5-pro) report cache_read but not
    // cache_creation. Infer it from the growth of cache_read between consecutive
    // calls: the delta is tokens newly added to cache. Split the API's cache_read
    // into actual cache_read (previously cached) + cache_creation (delta), keeping
    // per-call totals unchanged.
    let has_any_creation = turns
        .iter()
        .flat_map(|t| &t.calls)
        .any(|c| c.usage.cache_creation_input_tokens > 0);
    let has_any_read = turns
        .iter()
        .flat_map(|t| &t.calls)
        .any(|c| c.usage.cache_read_input_tokens > 0);
    if !has_any_creation && has_any_read {
        let mut max_cr: u64 = 0;
        for turn in &mut turns {
            for call in &mut turn.calls {
                let cr = call.usage.cache_read_input_tokens;
                if cr > max_cr {
                    call.usage.cache_creation_input_tokens = cr - max_cr;
                    call.usage.cache_read_input_tokens = max_cr;
                    call.usage = call.usage.finalize();
                    call.cost_usd = if call.usage.total == 0 {
                        0.0
                    } else {
                        pricing::cost_usd(&call.model, &call.usage)
                    };
                    max_cr = cr;
                }
            }
        }
    }

    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fork_jsonl_clones_with_fresh_ids_and_title() {
        // 两条链式记录（child.parentUuid 指向 parent.uuid）+ 原标题。
        let content = [
            r#"{"type":"user","sessionId":"old-sess","uuid":"u1","parentUuid":null,"message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"assistant","sessionId":"old-sess","uuid":"u2","parentUuid":"u1","message":{"role":"assistant","content":"hello"}}"#,
        ]
        .join("\n");
        let out = fork_jsonl(&content, "new-sess", "Chat fork");
        let rows: Vec<Value> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("valid json"))
            .collect();
        // 2 条原记录 + custom-title + agent-name。
        assert_eq!(rows.len(), 4);
        // 所有 sessionId 都改成新 id；不再出现旧 id。
        assert!(rows.iter().all(|r| r["sessionId"] == json!("new-sess")));
        assert!(!out.contains("old-sess"));
        // uuid 全部换新（不复用 u1/u2），且 child.parentUuid 仍指向 parent 的**新** uuid。
        let u1_new = rows[0]["uuid"].as_str().unwrap();
        let u2_parent = rows[1]["parentUuid"].as_str().unwrap();
        assert_ne!(u1_new, "u1");
        assert_ne!(rows[1]["uuid"].as_str().unwrap(), "u2");
        assert_eq!(u2_parent, u1_new, "parentUuid 应重映射到父记录的新 uuid");
        // 标题成对写入。
        assert_eq!(rows[2]["type"], json!("custom-title"));
        assert_eq!(rows[2]["customTitle"], json!("Chat fork"));
        assert_eq!(rows[3]["type"], json!("agent-name"));
        assert_eq!(rows[3]["agentName"], json!("Chat fork"));
    }

    #[test]
    fn fork_jsonl_before_user_turn_drops_edited_prompt_and_partial_answer() {
        let content = [
            r#"{"type":"user","sessionId":"old","uuid":"u1","parentUuid":null,"message":{"role":"user","content":"first"}}"#,
            r#"{"type":"assistant","sessionId":"old","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":"first answer"}}"#,
            r#"{"type":"user","sessionId":"old","uuid":"tool","parentUuid":"a1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
            r#"{"type":"user","sessionId":"old","uuid":"meta","parentUuid":"tool","isMeta":true,"message":{"role":"user","content":"injected"}}"#,
            r#"{"type":"user","sessionId":"old","uuid":"u2","parentUuid":"meta","message":{"role":"user","content":"edit me"}}"#,
            r#"{"type":"assistant","sessionId":"old","uuid":"a2","parentUuid":"u2","message":{"role":"assistant","content":"partial answer"}}"#,
            r#"{"type":"user","sessionId":"old","uuid":"stop","parentUuid":"a2","message":{"role":"user","content":"[Request interrupted by user]"}}"#,
        ]
        .join("\n");

        let out = fork_jsonl_before_user_turn(&content, "new", "Edited fork", 1)
            .expect("fork before second prompt");
        let rows: Vec<Value> = out
            .lines()
            .map(|line| serde_json::from_str(line).expect("valid json"))
            .collect();

        assert_eq!(rows.len(), 6, "4 prefix rows plus two title rows");
        assert!(out.contains("first answer"));
        assert!(out.contains("injected"));
        assert!(!out.contains("edit me"));
        assert!(!out.contains("partial answer"));
        assert!(!out.contains("Request interrupted"));
        assert!(rows.iter().all(|row| row["sessionId"] == json!("new")));
        let first_uuid = rows[0]["uuid"].as_str().unwrap();
        assert_eq!(rows[1]["parentUuid"], json!(first_uuid));
    }

    #[test]
    fn fork_jsonl_before_user_turn_requires_the_target_prompt() {
        let content = r#"{"type":"user","message":{"content":"only one"}}"#;
        let err = fork_jsonl_before_user_turn(content, "new", "fork", 1).unwrap_err();
        assert!(err.contains("找不到"));
    }

    #[test]
    fn fork_prompt_detection_matches_editable_user_messages() {
        assert!(is_editable_user_prompt_record(&json!({
            "type": "user",
            "message": { "content": "typed prompt" },
        })));
        assert!(is_editable_user_prompt_record(&json!({
            "type": "attachment",
            "attachment": { "type": "queued_command", "prompt": "queued prompt" },
        })));
        for record in [
            json!({
                "type": "user",
                "message": { "content": "[Request interrupted by user]" },
            }),
            json!({
                "type": "user",
                "message": { "content": "<local-command-caveat>internal</local-command-caveat>" },
            }),
            json!({
                "type": "user",
                "message": { "content": [{
                    "type": "tool_result", "tool_use_id": "t1", "content": "ok"
                }] },
            }),
            json!({
                "type": "user",
                "isMeta": true,
                "message": { "content": "injected" },
            }),
            json!({
                "type": "user",
                "isSidechain": true,
                "message": { "content": "subagent prompt" },
            }),
        ] {
            assert!(!is_editable_user_prompt_record(&record), "{record}");
        }
    }

    #[test]
    fn chat_command_fork_adds_fork_session_flag() {
        // btw 侧聊：续聊 + fork → --resume <id> --fork-session（继承上下文、派生新 id）。
        let cmd = ClaudeSource
            .chat_command(Some("abc-123"), "plan", None, None, true)
            .expect("claude chat command");
        let args = cmd.args();
        assert!(args.iter().any(|a| a == "--resume"));
        assert!(args.iter().any(|a| a == "abc-123"));
        assert!(args.iter().any(|a| a == "--fork-session"));
    }

    #[test]
    fn chat_command_no_fork_omits_fork_session_flag() {
        let cmd = ClaudeSource
            .chat_command(Some("abc-123"), "acceptEdits", None, None, false)
            .expect("claude chat command");
        assert!(!cmd.args().iter().any(|a| a == "--fork-session"));
    }

    #[test]
    fn chat_command_fork_without_session_has_no_fork_flag() {
        // 新开会话（无 session_id）无可派生对象 → 即便 fork=true 也不下发 --fork-session。
        let cmd = ClaudeSource
            .chat_command(None, "acceptEdits", None, None, true)
            .expect("claude chat command");
        assert!(!cmd.args().iter().any(|a| a == "--fork-session"));
        assert!(!cmd.args().iter().any(|a| a == "--resume"));
    }

    #[test]
    fn file_ref_quoted_path_becomes_file_block() {
        let (files, body) = extract_file_refs(
            "@\"/Users/wuchao/Downloads/仓库管理列表20260409163454.xlsx\"\nhi",
            None,
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, "file");
        assert_eq!(
            files[0].file_path.as_deref(),
            Some("/Users/wuchao/Downloads/仓库管理列表20260409163454.xlsx")
        );
        assert_eq!(body, "hi");
    }

    #[test]
    fn file_ref_directory_marked_is_dir() {
        // 真实存在的目录 → is_dir=Some(true)（历史会话的文件夹 chip 才显示文件夹图标）。
        let dir = std::env::temp_dir();
        let dir = dir.to_string_lossy();
        let dir = dir.trim_end_matches('/');
        let (files, _) = extract_file_refs(&format!("@\"{dir}\" hi"), None);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].is_dir, Some(true));
    }

    #[test]
    fn file_ref_nonexistent_path_not_marked_dir() {
        // 不存在的路径 → is_dir 留 None（退化成文件图标，不臆测）。
        let (files, _) = extract_file_refs("@\"/no/such/path/here.xyz\" hi", None);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].is_dir, None);
    }

    #[test]
    fn file_ref_unquoted_absolute_path() {
        let (files, body) = extract_file_refs(
            "@/Users/wuchao/Downloads/仓库管理列表20260409163454.xlsx\nhi",
            None,
        );
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_path.as_deref(),
            Some("/Users/wuchao/Downloads/仓库管理列表20260409163454.xlsx")
        );
        assert_eq!(body, "hi");
    }

    #[test]
    fn file_ref_multiple_files_one_message() {
        let (files, body) = extract_file_refs("@/a/one.txt @/b/two.md please review", None);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_path.as_deref(), Some("/a/one.txt"));
        assert_eq!(files[1].file_path.as_deref(), Some("/b/two.md"));
        assert!(body.contains("please review"));
    }

    #[test]
    fn file_ref_relative_repo_files() {
        // Claude `@` 选仓库文件的常见形态：无目录前缀的 `name.ext`，也要抽成 file 块。
        let (files, body) = extract_file_refs(
            "@main_driver.dart @package.json @analysis_options.yaml hi",
            None,
        );
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].file_path.as_deref(), Some("main_driver.dart"));
        assert_eq!(files[1].file_path.as_deref(), Some("package.json"));
        assert_eq!(files[2].file_path.as_deref(), Some("analysis_options.yaml"));
        assert_eq!(body, "hi");
    }

    #[test]
    fn file_ref_skips_plain_at_mention() {
        // 不像路径的 `@token`（无 `/`、非绝对路径）当普通文字，不抽成文件。
        let (files, body) = extract_file_refs("ping @teammate to review", None);
        assert!(files.is_empty());
        assert_eq!(body, "ping @teammate to review");
    }

    #[test]
    fn file_ref_in_mid_prompt_keeps_full_text_and_adds_tag() {
        let text = "分析@scripts/release/appstore-release.sh的执行过程和调用命令";
        let (files, body) = extract_file_refs(text, None);
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_path.as_deref(),
            Some("scripts/release/appstore-release.sh")
        );
        assert_eq!(body, text);
    }

    #[test]
    fn file_ref_at_prompt_end_becomes_tag_without_text_duplication() {
        let text = "我加了几个附件，回答我即可 @\"/tmp/one.xlsx\" @\"docs/two.md\"";
        let (files, body) = extract_file_refs(text, None);
        assert_eq!(files.len(), 2);
        assert_eq!(body, "我加了几个附件，回答我即可");
    }

    #[test]
    fn lift_file_refs_only_real_user_message() {
        let v = json!({
            "type": "user",
            "message": { "content": "@\"/tmp/report.xlsx\"\n看看这个" },
        });
        let msg = record_to_msg(&v).expect("user msg");
        assert_eq!(msg.blocks.len(), 2);
        assert_eq!(msg.blocks[0].kind, "file");
        assert_eq!(msg.blocks[0].file_path.as_deref(), Some("/tmp/report.xlsx"));
        assert_eq!(msg.blocks[1].kind, "text");
        assert_eq!(msg.blocks[1].text.as_deref(), Some("看看这个"));
    }

    #[test]
    fn record_mid_prompt_file_ref_has_tag_and_full_prompt() {
        let text = "分析@scripts/release/appstore-release.sh的执行过程和调用命令";
        let v = json!({
            "type": "user",
            "cwd": "/tmp",
            "message": { "content": text },
        });
        let msg = record_to_msg(&v).expect("user msg");
        assert_eq!(msg.blocks.len(), 2);
        assert_eq!(msg.blocks[0].kind, "file");
        assert_eq!(
            msg.blocks[0].file_path.as_deref(),
            Some("scripts/release/appstore-release.sh")
        );
        assert_eq!(msg.blocks[1].text.as_deref(), Some(text));
    }

    #[test]
    fn extracts_text_queued_command() {
        let v = json!({
            "type": "attachment",
            "attachment": { "type": "queued_command", "prompt": "改完看 readme" },
        });
        let blocks = queued_command_blocks(&v).expect("text prompt");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some("改完看 readme"));
    }

    #[test]
    fn extracts_queued_command_with_image() {
        // 带贴图的排队消息：prompt 是 text + image 数组，图片不能丢。
        let v = json!({
            "type": "attachment",
            "attachment": { "type": "queued_command", "prompt": [
                { "type": "text", "text": "[Image #10]" },
                { "type": "image", "source": {
                    "type": "base64", "media_type": "image/png", "data": "AAAA" } },
            ] },
        });
        let blocks = queued_command_blocks(&v).expect("text + image prompt");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[1].kind, "image");
        assert_eq!(
            blocks[1].image_src.as_deref(),
            Some("data:image/png;base64,AAAA"),
        );
    }

    // ---- classify_meta_kind --------------------------------------------

    fn text_blocks(s: &str) -> Vec<Block> {
        vec![text_block("text", s)]
    }

    #[test]
    fn meta_kind_flags_compaction_summary() {
        let v = json!({
            "type": "user",
            "isCompactSummary": true,
            "isVisibleInTranscriptOnly": true,
            "message": { "content": "This session is being continued..." },
        });
        let blocks = text_blocks("This session is being continued...");
        assert_eq!(classify_meta_kind(&v, &blocks).as_deref(), Some("compact"));
    }

    #[test]
    fn meta_kind_flags_compaction_summary_by_preamble_without_flag() {
        // GUI chat 的 headless stream-json 续聊摘要事件不带 isCompactSummary flag，
        // 只能靠 Claude Code 固定的续聊开场白识别 —— 否则会渲染成「Me」气泡。
        let v = json!({ "type": "user", "message": { "content": "" } });
        let blocks = text_blocks(
            "This session is being continued from a previous conversation that ran out of context. The summary below...",
        );
        assert_eq!(classify_meta_kind(&v, &blocks).as_deref(), Some("compact"));
    }

    #[test]
    fn meta_kind_flags_source_tool_use_id() {
        // sourceToolUseID 表示此 user 记录是工具执行后注入的（技能正文等），
        // 即使不带 isMeta flag（stream-json 常见）也应归为 meta。
        let v = json!({
            "type": "user",
            "sourceToolUseID": "toolu_01Csnf8gVU635bmuM3tHahNw",
            "message": { "content": "# Git 提交推送命令\n\n自动提交..." },
        });
        let blocks = text_blocks("# Git 提交推送命令\n\n自动提交...");
        assert_eq!(classify_meta_kind(&v, &blocks).as_deref(), Some("meta"));
    }

    #[test]
    fn meta_kind_flags_skill_injection_by_preamble() {
        // 技能注入的固定开场白兜底（连 sourceToolUseID 都没有时）。
        let v = json!({ "type": "user", "message": { "content": "" } });
        let blocks = text_blocks(
            "Base directory for this skill: /private/tmp/claude-501/bundled-skills/2.1.198/xxx/claude-api\n\n# Building LLM",
        );
        assert_eq!(classify_meta_kind(&v, &blocks).as_deref(), Some("meta"));
    }

    #[test]
    fn read_reorders_compact_summary_after_the_compact_command() {
        // Claude Code 把续聊摘要记录写在文件**开头**（早于触发它的 /compact 命令），但摘要的
        // timestamp 其实晚于命令（压缩完成时刻）。read() 应按 timestamp 稳定排序把摘要归位 ——
        // 否则「结果」会渲染到用户的 /compact 之上。
        let p = write_temp(
            "compact-order.jsonl",
            &[
                r#"{"type":"user","isCompactSummary":true,"timestamp":"2026-06-29T09:32:42.834Z","message":{"content":"This session is being continued from a previous conversation that ran out of context. The summary below..."}}"#,
                r#"{"type":"user","timestamp":"2026-06-29T09:31:55.402Z","message":{"content":"<command-name>/compact</command-name>\n<command-message>compact</command-message>\n<command-args></command-args>"}}"#,
                r#"{"type":"user","timestamp":"2026-06-29T09:32:42.953Z","message":{"content":"<local-command-stdout>Compacted </local-command-stdout>"}}"#,
            ],
        );
        let msgs = read(p.to_str().unwrap()).unwrap();
        let cmd = msgs
            .iter()
            .position(|m| {
                m.blocks.iter().any(|b| {
                    b.text
                        .as_deref()
                        .unwrap_or("")
                        .contains("<command-name>/compact")
                })
            })
            .expect("/compact command record");
        let summary = msgs
            .iter()
            .position(|m| m.meta_kind.as_deref() == Some("compact"))
            .expect("compact summary record");
        assert!(
            cmd < summary,
            "压缩摘要 (idx {summary}) 应排在 /compact 命令 (idx {cmd}) 之后"
        );
    }

    #[test]
    fn read_revives_context_local_command_as_command_output() {
        // on-disk 的 /context 一轮 = user 命令 + system/local_command(带壳 stdout) +
        // assistant「No response requested.」占位。read() 应：把 local_command 当 user 记录
        // 交给既有逻辑 → command-output 折叠块（前端 contextUsageOf 再升级成卡片），且**不**
        // 造 model=<synthetic> 的 assistant（否则带歪模型选择器）；并丢掉「No response requested.」。
        let p = write_temp(
            "context-local-command.jsonl",
            &[
                r#"{"type":"user","timestamp":"2026-06-29T10:00:07.214Z","message":{"content":"<command-name>/context</command-name>\n<command-message>context</command-message>\n<command-args></command-args>"}}"#,
                r#"{"type":"system","subtype":"local_command","timestamp":"2026-06-29T10:00:18.715Z","content":"<local-command-stdout>## Context Usage\n\n**Model:** claude-opus-4-8[1m]\n**Tokens:** 26.8k / 400k (7%)\n\n### Estimated usage by category\n\n| Category | Tokens | Percentage |\n|----------|--------|------------|\n| Messages | 10k | 2.5% |\n| Free space | 339.9k | 85.0% |</local-command-stdout>"}"#,
                r#"{"type":"assistant","timestamp":"2026-06-29T10:16:19.000Z","message":{"role":"assistant","model":"<synthetic>","content":[{"type":"text","text":"No response requested."}]}}"#,
            ],
        );
        let msgs = read(p.to_str().unwrap()).unwrap();
        let cmd = msgs
            .iter()
            .position(|m| {
                m.blocks.iter().any(|b| {
                    b.text
                        .as_deref()
                        .unwrap_or("")
                        .contains("<command-name>/context")
                })
            })
            .expect("/context command record");
        let card = msgs
            .iter()
            .position(|m| {
                m.blocks
                    .iter()
                    .any(|b| b.text.as_deref().unwrap_or("").contains("## Context Usage"))
            })
            .expect("revived /context breakdown record");
        // command-output 折叠块（user 角色 + meta_kind），正文保留 <local-command-stdout> 外壳
        // （前端 cleanMetaText 去壳后再渲染 / 升级成卡片）。绝不能是 model=<synthetic> 的 assistant。
        assert_eq!(msgs[card].role, "user");
        assert_eq!(msgs[card].meta_kind.as_deref(), Some("command-output"));
        assert!(msgs
            .iter()
            .all(|m| m.model.as_deref() != Some("<synthetic>")));
        assert!(
            cmd < card,
            "卡片块 (idx {card}) 应排在 /context 命令 (idx {cmd}) 之后"
        );
        // 「No response requested.」占位被丢弃，不残留任何气泡。
        assert!(
            !msgs.iter().any(|m| {
                m.blocks
                    .iter()
                    .any(|b| b.text.as_deref().map(str::trim) == Some("No response requested."))
            }),
            "synthetic「No response requested.」占位不该出现在离线回看里",
        );
    }

    #[test]
    fn meta_kind_flags_skill_injection_ismeta() {
        let v = json!({ "type": "user", "isMeta": true });
        let blocks = text_blocks("Base directory for this skill: /x/y");
        assert_eq!(classify_meta_kind(&v, &blocks).as_deref(), Some("meta"));
    }

    #[test]
    fn meta_kind_flags_task_notification_by_origin() {
        let v = json!({
            "type": "user",
            "promptSource": "system",
            "origin": { "kind": "task-notification" },
        });
        let blocks = text_blocks("<task-notification>\n<task-id>x</task-id>\n</task-notification>");
        assert_eq!(
            classify_meta_kind(&v, &blocks).as_deref(),
            Some("task-notification"),
        );
    }

    #[test]
    fn meta_kind_flags_command_output_by_content() {
        // 老版本 CC：没有 promptSource/origin，只有内容标签兜底。
        let v = json!({ "type": "user" });
        let blocks = text_blocks("<local-command-stdout>Terminal setup...</local-command-stdout>");
        assert_eq!(
            classify_meta_kind(&v, &blocks).as_deref(),
            Some("command-output"),
        );
        let blocks = text_blocks("<bash-stdout>remote: ...</bash-stdout>");
        assert_eq!(
            classify_meta_kind(&v, &blocks).as_deref(),
            Some("command-output"),
        );
    }

    #[test]
    fn meta_kind_flags_teammate_message_by_content() {
        // 多 agent 协作：对方会话发来的消息无 flag，只能看正文前缀 / 标签。
        let v = json!({ "type": "user" });
        let blocks = text_blocks(
            "Another Claude session sent a message:\n<teammate-message teammate_id=\"x\" color=\"blue\">\n{\"type\":\"idle_notification\"}\n</teammate-message>",
        );
        assert_eq!(
            classify_meta_kind(&v, &blocks).as_deref(),
            Some("teammate-message"),
        );
    }

    #[test]
    fn meta_kind_flags_queued_task_notification_attachment() {
        // 处理过程中到达的通知被「排队」成 attachment（commandMode==task-notification），
        // 不是常规 type:user 记录 —— 也不能渲染成 Me。
        let v = json!({
            "type": "attachment",
            "attachment": {
                "type": "queued_command",
                "commandMode": "task-notification",
                "prompt": "<task-notification>\n<task-id>bz2lxppsz</task-id>\n<status>completed</status>\n</task-notification>",
            },
        });
        let blocks =
            text_blocks("<task-notification>\n<task-id>bz2lxppsz</task-id>\n</task-notification>");
        assert_eq!(
            classify_meta_kind(&v, &blocks).as_deref(),
            Some("task-notification"),
        );
    }

    #[test]
    fn meta_kind_keeps_queued_user_prompt_as_me() {
        // 用户在 Claude 处理时手敲的排队消息 commandMode == "prompt" → 仍是 Me。
        let v = json!({
            "type": "attachment",
            "attachment": {
                "type": "queued_command",
                "commandMode": "prompt",
                "prompt": "这个是app项目",
            },
        });
        let blocks = text_blocks("这个是app项目");
        assert_eq!(classify_meta_kind(&v, &blocks), None);
    }

    #[test]
    fn meta_kind_leaves_real_user_messages_alone() {
        // 真正手敲的消息 + 用户主动发起的 slash / bash 命令 → None（仍是「Me」）。
        let typed = json!({
            "type": "user",
            "promptSource": "typed",
            "origin": { "kind": "human" },
        });
        assert_eq!(
            classify_meta_kind(&typed, &text_blocks("pull了；继续")),
            None
        );

        let no_markers = json!({ "type": "user" });
        assert_eq!(
            classify_meta_kind(&no_markers, &text_blocks("hello there")),
            None
        );

        // slash 命令调用是用户主动行为，保持 Me。
        let slash = json!({ "type": "user" });
        let blocks = text_blocks(
            "<command-message>dm-watch</command-message>\n<command-name>/dm-watch</command-name>",
        );
        assert_eq!(classify_meta_kind(&slash, &blocks), None);

        // 用户 `!git push` 的输入侧（输出侧才算 command-output）。
        let bash_in = json!({ "type": "user" });
        assert_eq!(
            classify_meta_kind(&bash_in, &text_blocks("<bash-input>git push</bash-input>")),
            None,
        );
    }

    #[test]
    fn ignores_non_queued_attachments() {
        // hook_success / task_reminder / diagnostics 等 attachment 不是用户消息
        let v = json!({
            "type": "attachment",
            "attachment": { "type": "hook_success", "content": "OK" },
        });
        assert!(queued_command_blocks(&v).is_none());
    }

    #[test]
    fn ignores_blank_queued_prompt() {
        let v = json!({
            "type": "attachment",
            "attachment": { "type": "queued_command", "prompt": "   " },
        });
        assert!(queued_command_blocks(&v).is_none());
    }

    // ---- usage_summary --------------------------------------------------

    use std::io::Write;

    fn write_temp(name: &str, lines: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("csv-claude-usage-tests");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join(name);
        let mut f = fs::File::create(&p).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        p
    }

    #[test]
    fn usage_sums_input_output_cache_across_assistant_messages() {
        let p = write_temp(
            "sum.jsonl",
            &[
                r#"{"type":"user","message":{"content":"hi"}}"#,
                r#"{"type":"assistant","message":{"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":0}}}"#,
                r#"{"type":"assistant","message":{"usage":{"input_tokens":3,"output_tokens":7,"cache_creation_input_tokens":0,"cache_read_input_tokens":100}}}"#,
            ],
        );
        let u = usage_summary(&p).unwrap();
        assert_eq!(u.input_tokens, 13);
        assert_eq!(u.output_tokens, 12);
        assert_eq!(u.cache_creation_input_tokens, 100);
        assert_eq!(u.cache_read_input_tokens, 100);
        assert_eq!(u.reasoning_output_tokens, 0);
        assert_eq!(u.total, 225);
    }

    #[test]
    fn usage_ignores_lines_without_usage() {
        let p = write_temp(
            "no-usage.jsonl",
            &[
                r#"{"type":"user","message":{"content":"hi"}}"#,
                r#"{"type":"system","content":"x"}"#,
            ],
        );
        assert_eq!(usage_summary(&p).unwrap(), UsageSummary::default());
    }

    #[test]
    fn usage_handles_missing_subfields_as_zero() {
        let p = write_temp(
            "partial.jsonl",
            &[
                // 只有 output_tokens，其他字段缺失 —— 不应该挂
                r#"{"type":"assistant","message":{"usage":{"output_tokens":42}}}"#,
            ],
        );
        let u = usage_summary(&p).unwrap();
        assert_eq!(u.output_tokens, 42);
        assert_eq!(u.total, 42);
    }

    #[test]
    fn usage_returns_default_when_file_missing() {
        let p = std::path::PathBuf::from("/tmp/csv-claude-usage-tests/nonexistent.jsonl");
        assert_eq!(usage_summary(&p).unwrap(), UsageSummary::default());
    }

    #[test]
    fn scan_uses_last_cwd_after_cd() {
        let p = write_temp(
            "cwd-moved.jsonl",
            &[
                r#"{"type":"user","cwd":"C:\\Users\\BLL","message":{"content":"start"},"timestamp":"2025-01-01T00:00:00.000Z"}"#,
                r#"{"type":"assistant","cwd":"C:\\Users\\BLL","message":{"content":[{"type":"text","text":"Already in C:\\Users\\BLL."}]}}"#,
                r#"{"type":"user","cwd":"D:\\ZLSYSproject","message":{"content":"/cd D:\\ZLSYSproject"},"timestamp":"2025-01-01T00:00:01.000Z"}"#,
                r#"{"type":"assistant","cwd":"D:\\ZLSYSproject","message":{"content":[{"type":"text","text":"Moved to D:\\ZLSYSproject"}]}}"#,
            ],
        );
        let meta = scan(&p);
        assert_eq!(meta.cwd.as_deref(), Some(r#"D:\ZLSYSproject"#));
    }

    #[test]
    fn json_str_field_prefix_extracts_and_unescapes() {
        let line = br#"{"parentUuid":"x","userType":"external","cwd":"C:\\Users\\BLL","type":"user","message":{"content":"hi"}}"#;
        assert_eq!(json_str_field_prefix(line, "type").as_deref(), Some("user"));
        // 转义还原 + 不被 "userType" 误命中
        assert_eq!(
            json_str_field_prefix(line, "cwd").as_deref(),
            Some(r#"C:\Users\BLL"#)
        );
        // 字段不存在
        assert_eq!(json_str_field_prefix(line, "nope"), None);
        // 字段值落在前缀窗口之外（被截断）→ None，调用方回退整行解析
        let big = br#"{"cwd":"/x","type":"user","message":"AAAA"#; // 未闭合
        assert_eq!(json_str_field_prefix(&big[..20], "message"), None);
    }

    #[test]
    fn list_projects_uses_last_cwd_for_display_path() {
        let root = std::env::temp_dir().join("csv-claude-project-cwd-tests");
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("moved-project");
        fs::create_dir_all(&proj).unwrap();
        let session = proj.join("session-1.jsonl");
        fs::write(
            &session,
            [
                r#"{"type":"user","cwd":"C:\\Users\\BLL","message":{"content":"start"}}"#,
                r#"{"type":"user","cwd":"D:\\ZLSYSproject","message":{"content":"/cd D:\\ZLSYSproject"}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let projects = list_projects_in(&root).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].display_path, r#"D:\ZLSYSproject"#);
    }

    #[test]
    #[ignore = "manual full-scan; reads every Claude JSONL on disk"]
    fn dedup_full_claude_scan() {
        let src = ClaudeSource;
        let projects = src.list_projects(false, false).unwrap();
        let mut agg = crate::stats::aggregate::Aggregator::new();
        for p in &projects {
            let sessions = src.discover_stats_sessions(&p.dir_name).unwrap_or_default();
            for s in sessions {
                let turns = read_turns(std::path::Path::new(&s.path));
                agg.feed_session(&crate::stats::aggregate::SessionFeed {
                    agent: "claude",
                    project_dir_name: &p.dir_name,
                    project_display: &p.display_path,
                    session_id: &s.id,
                    path: &s.path,
                    title: &s.title,
                    last_modified: s.modified,
                    message_count: s.message_count,
                    turns: &turns,
                });
            }
        }
        let snap = agg.snapshot("claude");
        eprintln!("\n=== FULL CLAUDE SCAN (with dedup + subagents) ===");
        eprintln!("sessions: {}", snap.session_count);
        eprintln!("calls: {}", snap.call_count);
        eprintln!("cost: ${:.2}", snap.cost_usd);
        eprintln!(
            "input: {} ({:.1}M)",
            snap.usage.input_tokens,
            snap.usage.input_tokens as f64 / 1e6
        );
        eprintln!(
            "output: {} ({:.1}M)",
            snap.usage.output_tokens,
            snap.usage.output_tokens as f64 / 1e6
        );
        eprintln!(
            "cache_read: {} ({:.1}M)",
            snap.usage.cache_read_input_tokens,
            snap.usage.cache_read_input_tokens as f64 / 1e6
        );
        eprintln!(
            "cache_write: {} ({:.1}M)",
            snap.usage.cache_creation_input_tokens,
            snap.usage.cache_creation_input_tokens as f64 / 1e6
        );
        eprintln!("\ndaily activity (top 15 by cost):");
        let mut daily = snap.daily_activity.clone();
        daily.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for d in daily.iter().take(15) {
            eprintln!("  {}  ${:>7.2}  calls={}", d.date, d.cost_usd, d.call_count);
        }
    }

    #[test]
    #[ignore = "manual; set CLAUDE_DEDUP_FIXTURE=<path>.jsonl to run"]
    fn dedup_verify_real_file() {
        let Ok(path) = std::env::var("CLAUDE_DEDUP_FIXTURE") else {
            return;
        };
        let turns = read_turns(std::path::Path::new(&path));
        let total: usize = turns.iter().map(|t| t.calls.len()).sum();
        let uniq: std::collections::HashSet<&String> = turns
            .iter()
            .flat_map(|t| &t.calls)
            .filter_map(|c| c.message_id.as_ref())
            .collect();
        eprintln!("\nfile: {path}");
        eprintln!(
            "  turns: {} calls(pre-dedup): {} unique msg-ids: {}",
            turns.len(),
            total,
            uniq.len()
        );
        let mut agg = crate::stats::aggregate::Aggregator::new();
        agg.feed_session(&crate::stats::aggregate::SessionFeed {
            agent: "claude",
            project_dir_name: "p",
            project_display: "/p",
            session_id: "s",
            path: &path,
            title: "t",
            last_modified: 1,
            message_count: 0,
            turns: &turns,
        });
        let s = agg.snapshot("test");
        eprintln!(
            "aggregator: call_count={} cost=${:.2} input={} output={} cache_read={}",
            s.call_count,
            s.cost_usd,
            s.usage.input_tokens,
            s.usage.output_tokens,
            s.usage.cache_read_input_tokens
        );
    }

    // ---- subagent fold --------------------------------------------------

    #[test]
    fn scan_folds_subagent_into_parent_session_id() {
        // sidebar 已经把 subagent 排除在 session 列表外（list_sessions 只读
        // <project>/*.jsonl），但 stats 走的是 scan() —— 这里要保证 subagent 用
        // 父 UUID 作为 session_id，让聚合器把它们合到父 session 下，避免一个
        // 概念两个数（sidebar 198 / stats 298）。
        let p = std::path::PathBuf::from(
            "/x/.claude/projects/-Users-x-app/abc123-uuid/subagents/agent-foo.jsonl",
        );
        assert!(is_subagent_path(&p));
        let meta = scan(&p);
        assert_eq!(
            meta.id, "abc123-uuid",
            "subagent session id should be parent uuid"
        );
    }

    #[test]
    fn read_turns_coalesces_streaming_duplicates_by_message_id() {
        // Third-party models (e.g. mimo-v2.5-pro) write multiple assistant lines
        // per API call sharing the same message.id — streaming intermediates have
        // input_tokens=0/output_tokens=0, only the final entry has real usage.
        // read_turns must coalesce them so the aggregator's cross-file dedup
        // keeps the real token counts, not the 0 placeholders.
        let dir = std::env::temp_dir().join("csv-claude-coalesce-tests");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("coalesce.jsonl");
        let lines = [
            r#"{"type":"user","message":{"content":"hello"},"timestamp":"2025-01-01T00:00:00.000Z"}"#,
            // streaming intermediate — 0 usage, same id
            r#"{"type":"assistant","message":{"id":"msg_dup","model":"mimo-v2.5-pro","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":0,"output_tokens":0}},"timestamp":"2025-01-01T00:00:01.000Z"}"#,
            // final entry — real usage, same id, has tool_use
            r#"{"type":"assistant","message":{"id":"msg_dup","model":"mimo-v2.5-pro","content":[{"type":"text","text":"done"},{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":500,"output_tokens":200,"cache_creation_input_tokens":2000,"cache_read_input_tokens":10000}},"timestamp":"2025-01-01T00:00:02.000Z"}"#,
        ];
        std::fs::write(&p, lines.join("\n")).unwrap();
        let turns = read_turns(&p);
        assert_eq!(turns.len(), 1, "one user message = one turn");
        assert_eq!(
            turns[0].calls.len(),
            1,
            "same message_id must coalesce into 1 call"
        );
        let call = &turns[0].calls[0];
        assert_eq!(call.usage.input_tokens, 500);
        assert_eq!(call.usage.output_tokens, 200);
        assert_eq!(call.usage.cache_read_input_tokens, 10000);
        assert!(
            call.usage.total > 0,
            "total must be non-zero after coalescing"
        );
        assert!(
            call.tools.contains(&"Bash".to_string()),
            "tools from later entry must be merged"
        );
    }

    #[test]
    fn read_turns_infers_cache_creation_from_cache_read_growth() {
        // Third-party models report cache_creation=0 but cache_read grows between
        // calls. read_turns should split cache_read into actual read (previously
        // existing) + inferred creation (delta), preserving per-call totals.
        let dir = std::env::temp_dir().join("csv-claude-infer-tests");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("infer-cc.jsonl");
        let lines = [
            r#"{"type":"user","message":{"content":"q1"},"timestamp":"2025-01-01T00:00:00.000Z"}"#,
            r#"{"type":"assistant","message":{"id":"msg_a","model":"mimo-v2.5-pro","content":[{"type":"text","text":"a1"}],"usage":{"input_tokens":76524,"output_tokens":136,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}},"timestamp":"2025-01-01T00:00:01.000Z"}"#,
            r#"{"type":"user","message":{"content":"q2"},"timestamp":"2025-01-01T00:00:02.000Z"}"#,
            r#"{"type":"assistant","message":{"id":"msg_b","model":"mimo-v2.5-pro","content":[{"type":"text","text":"a2"}],"usage":{"input_tokens":275,"output_tokens":171,"cache_creation_input_tokens":0,"cache_read_input_tokens":76480}},"timestamp":"2025-01-01T00:00:03.000Z"}"#,
            r#"{"type":"user","message":{"content":"q3"},"timestamp":"2025-01-01T00:00:04.000Z"}"#,
            r#"{"type":"assistant","message":{"id":"msg_c","model":"mimo-v2.5-pro","content":[{"type":"text","text":"a3"}],"usage":{"input_tokens":3319,"output_tokens":150,"cache_creation_input_tokens":0,"cache_read_input_tokens":78144}},"timestamp":"2025-01-01T00:00:05.000Z"}"#,
        ];
        std::fs::write(&p, lines.join("\n")).unwrap();
        let turns = read_turns(&p);
        assert_eq!(turns.len(), 3);

        // Call 0: cache_read=0, no inference (nothing grew)
        let c0 = &turns[0].calls[0];
        assert_eq!(c0.usage.cache_creation_input_tokens, 0);
        assert_eq!(c0.usage.cache_read_input_tokens, 0);
        assert_eq!(c0.usage.input_tokens, 76524);

        // Call 1: cache_read grew 0→76480; split: creation=76480, read=0
        let c1 = &turns[1].calls[0];
        assert_eq!(c1.usage.cache_creation_input_tokens, 76480);
        assert_eq!(c1.usage.cache_read_input_tokens, 0);
        assert_eq!(c1.usage.input_tokens, 275, "input_tokens unchanged");
        // Per-call total preserved: 275+171+76480+0 = 76926
        assert_eq!(c1.usage.total, 275 + 171 + 76480);

        // Call 2: cache_read grew 76480→78144; split: creation=1664, read=76480
        let c2 = &turns[2].calls[0];
        assert_eq!(c2.usage.cache_creation_input_tokens, 1664);
        assert_eq!(c2.usage.cache_read_input_tokens, 76480);
        assert_eq!(c2.usage.input_tokens, 3319);
        assert_eq!(c2.usage.total, 3319 + 150 + 1664 + 76480);
    }

    #[test]
    fn scan_title_skips_injected_user_records() {
        // /dm-watch 会话开头是 isMeta 的 skill 注入；标题应回落到首条真实用户消息，
        // 而不是 "Base directory for this skill: …"。
        let dir = std::env::temp_dir().join("csv-claude-title-tests");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("injected-first.jsonl");
        let lines = [
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill: /Users/x/.claude/skills/dm-watch"}]},"timestamp":"2025-01-01T00:00:00.000Z"}"#,
            r#"{"type":"user","message":{"role":"user","content":"真正的第一句话"},"timestamp":"2025-01-01T00:00:01.000Z"}"#,
        ];
        std::fs::write(&p, lines.join("\n")).unwrap();
        let meta = scan(&p);
        assert_eq!(meta.title, "真正的第一句话");
    }

    #[test]
    fn scan_keeps_top_level_session_id_unchanged() {
        let p = std::path::PathBuf::from("/x/.claude/projects/-Users-x-app/abc123-uuid.jsonl");
        assert!(!is_subagent_path(&p));
        let meta = scan(&p);
        assert_eq!(meta.id, "abc123-uuid");
    }

    // ---- parse_chat_line: stream-json 事件归一 ----

    #[test]
    fn parse_chat_line_assistant_event_becomes_message() {
        let line = r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","model":"claude-opus-4","content":[{"type":"text","text":"hi there"}]},"session_id":"s1"}"#;
        match parse_chat_line(line) {
            ChatEvent::Message(m) => {
                assert_eq!(m.role, "assistant");
                assert_eq!(m.model.as_deref(), Some("claude-opus-4"));
                assert_eq!(m.blocks.len(), 1);
                assert_eq!(m.blocks[0].kind, "text");
                assert_eq!(m.blocks[0].text.as_deref(), Some("hi there"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn parse_chat_line_user_tool_result_becomes_message_not_meta() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"session_id":"s1"}"#;
        match parse_chat_line(line) {
            ChatEvent::Message(m) => {
                assert_eq!(m.role, "user");
                assert!(m.meta_kind.is_none(), "tool_result user must not be meta");
                assert_eq!(m.blocks[0].kind, "tool_result");
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn parse_chat_line_system_init_carries_session_id() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","apiKeySource":"none","tools":[]}"#;
        match parse_chat_line(line) {
            ChatEvent::Init {
                session_id,
                api_key_source,
            } => {
                assert_eq!(session_id.as_deref(), Some("abc-123"));
                assert_eq!(api_key_source.as_deref(), Some("none"));
            }
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn parse_chat_line_non_init_system_events_are_ignored() {
        for line in [
            r#"{"type":"system","subtype":"status","status":"requesting"}"#,
            r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":25}"#,
            r#"{"type":"system","subtype":"hook_started","hook_name":"SessionStart:startup"}"#,
        ] {
            assert!(matches!(parse_chat_line(line), ChatEvent::Ignore));
        }
    }

    #[test]
    fn parse_chat_line_result_success_is_ok_with_usage() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":2}}"#;
        match parse_chat_line(line) {
            ChatEvent::Result { ok, usage } => {
                assert!(ok);
                let u = usage.expect("usage present");
                assert_eq!(u.input_tokens, 10);
                assert_eq!(u.output_tokens, 5);
                assert_eq!(u.cache_read_input_tokens, 2);
                assert_eq!(u.total, 17);
            }
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn parse_chat_line_result_error_is_not_ok() {
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
        match parse_chat_line(line) {
            ChatEvent::Result { ok, .. } => assert!(!ok),
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn parse_chat_line_ignores_unknown_and_garbage() {
        assert!(matches!(parse_chat_line("not json"), ChatEvent::Ignore));
        assert!(matches!(parse_chat_line(""), ChatEvent::Ignore));
        assert!(matches!(
            parse_chat_line(r#"{"type":"stream_event","event":{}}"#),
            ChatEvent::Ignore
        ));
    }

    // ---- §10.6 token 级流式：stream_event → ChatEvent::Delta ----

    #[test]
    fn stream_content_block_start_is_delta_start_with_kind() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text"}},"session_id":"s1"}"#;
        match parse_chat_line(line) {
            ChatEvent::Delta(d) => {
                assert_eq!(d.index, 1);
                assert_eq!(d.phase, "start");
                assert_eq!(d.kind.as_deref(), Some("text"));
                assert!(d.text.is_none());
            }
            _ => panic!("expected Delta start"),
        }
    }

    #[test]
    fn stream_text_delta_carries_text_fragment() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"1\n2\n"}}}"#;
        match parse_chat_line(line) {
            ChatEvent::Delta(d) => {
                assert_eq!(d.index, 1);
                assert_eq!(d.phase, "delta");
                assert_eq!(d.kind.as_deref(), Some("text"));
                assert_eq!(d.text.as_deref(), Some("1\n2\n"));
            }
            _ => panic!("expected Delta delta"),
        }
    }

    #[test]
    fn stream_thinking_delta_carries_thinking_fragment() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}}"#;
        match parse_chat_line(line) {
            ChatEvent::Delta(d) => {
                assert_eq!(d.kind.as_deref(), Some("thinking"));
                assert_eq!(d.text.as_deref(), Some("hmm"));
            }
            _ => panic!("expected Delta from thinking_delta"),
        }
    }

    #[test]
    fn stream_signature_and_input_json_deltas_are_ignored() {
        // 签名 / 工具入参增量对打字机无用 —— 不产 Delta（权威 assistant 记录会定稿）。
        assert!(matches!(
            parse_chat_line(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"Eq=="}}}"#
            ),
            ChatEvent::Ignore
        ));
        assert!(matches!(
            parse_chat_line(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}}"#
            ),
            ChatEvent::Ignore
        ));
    }

    #[test]
    fn stream_content_block_stop_is_delta_stop() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1}}"#;
        match parse_chat_line(line) {
            ChatEvent::Delta(d) => {
                assert_eq!(d.index, 1);
                assert_eq!(d.phase, "stop");
            }
            _ => panic!("expected Delta stop"),
        }
    }

    #[test]
    fn stream_message_lifecycle_events_are_ignored() {
        for et in ["message_start", "message_delta", "message_stop"] {
            let line = format!(r#"{{"type":"stream_event","event":{{"type":"{et}"}}}}"#);
            assert!(
                matches!(parse_chat_line(&line), ChatEvent::Ignore),
                "{et} should be Ignore"
            );
        }
    }

    #[test]
    fn assistant_record_still_parses_as_message_alongside_streaming() {
        // 关键：开了 --include-partial-messages 后，权威 assistant 记录照旧到达并解析成 Message。
        let line = r#"{"type":"assistant","message":{"id":"m","role":"assistant","model":"claude-opus-4-8","content":[{"type":"text","text":"done"}]},"session_id":"s1"}"#;
        assert!(matches!(parse_chat_line(line), ChatEvent::Message(_)));
    }

    // ---- 交互式工具权限：control_request / can_use_tool → ChatEvent::Permission ----

    #[test]
    fn chat_command_enables_stdio_permission_prompt() {
        // 没有这个 flag，被门控的工具不会发 can_use_tool 请求 —— GUI 弹框就无从触发。
        let cmd = ClaudeSource
            .chat_command(None, "default", None, None, false)
            .expect("claude chat command");
        let args = cmd.args();
        let pos = args
            .iter()
            .position(|a| a == "--permission-prompt-tool")
            .expect("--permission-prompt-tool present");
        assert_eq!(args.get(pos + 1).map(String::as_str), Some("stdio"));
    }

    #[test]
    fn parse_chat_line_can_use_tool_becomes_permission() {
        let line = r#"{"type":"control_request","request_id":"req-7","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf build"},"description":"Delete build dir","permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash"}],"behavior":"allow","destination":"localSettings"}]}}"#;
        match parse_chat_line(line) {
            ChatEvent::Permission(p) => {
                assert_eq!(p.request_id, "req-7");
                assert_eq!(p.tool_name, "Bash");
                assert_eq!(
                    p.input.get("command").and_then(|c| c.as_str()),
                    Some("rm -rf build")
                );
                assert_eq!(p.description.as_deref(), Some("Delete build dir"));
                assert!(p
                    .permission_suggestions
                    .as_ref()
                    .is_some_and(|s| s.is_array()));
            }
            _ => panic!("expected Permission"),
        }
    }

    #[test]
    fn parse_chat_line_other_control_request_subtypes_ignored() {
        // 无 MCP/hook 时只有 can_use_tool 会出现；其它 control_request（如 initialize）不弹框。
        assert!(matches!(
            parse_chat_line(
                r#"{"type":"control_request","request_id":"i","request":{"subtype":"initialize"}}"#
            ),
            ChatEvent::Ignore
        ));
        // 缺 tool_name 的畸形 can_use_tool 也安全降级为 Ignore（不构造半成品请求）。
        assert!(matches!(
            parse_chat_line(
                r#"{"type":"control_request","request_id":"i","request":{"subtype":"can_use_tool","input":{}}}"#
            ),
            ChatEvent::Ignore
        ));
    }

    // ---- §10.1 slash 指令磁盘发现 ----

    #[test]
    fn parse_frontmatter_extracts_scalars_and_strips_quotes() {
        let fm = parse_frontmatter("---\ndescription: \"Do a thing\"\nname: foo\n---\nbody\n");
        assert_eq!(
            fm.get("description").map(String::as_str),
            Some("Do a thing")
        );
        assert_eq!(fm.get("name").map(String::as_str), Some("foo"));
    }

    #[test]
    fn parse_frontmatter_empty_when_no_block() {
        assert!(parse_frontmatter("no frontmatter here\n# Title").is_empty());
    }

    #[test]
    fn strip_frontmatter_returns_body_after_close() {
        assert_eq!(
            strip_frontmatter("---\na: b\n---\nhello\nworld\n").trim(),
            "hello\nworld"
        );
        assert_eq!(strip_frontmatter("plain body").trim(), "plain body");
    }

    #[test]
    fn md_description_prefers_frontmatter_then_first_body_line() {
        let dir = std::env::temp_dir().join("csv-claude-slash-desc");
        let _ = fs::create_dir_all(&dir);
        let a = dir.join("a.md");
        fs::write(
            &a,
            "---\ndescription: From frontmatter\n---\n# Heading\nbody",
        )
        .unwrap();
        assert_eq!(md_description(&a).as_deref(), Some("From frontmatter"));

        let b = dir.join("b.md");
        fs::write(&b, "# Heading\n\nFirst real line\nsecond").unwrap();
        assert_eq!(md_description(&b).as_deref(), Some("First real line"));
    }

    #[test]
    fn scan_commands_dir_namespaces_nested_and_skips_dups() {
        let root = std::env::temp_dir().join("csv-claude-slash-scan");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("git")).unwrap();
        fs::write(
            root.join("review.md"),
            "---\ndescription: Review code\nargument-hint: '[--wait|--background] [--base <ref>]'\n---\n",
        )
        .unwrap();
        fs::write(root.join("git").join("commit.md"), "Make a commit").unwrap();
        fs::write(root.join("notes.txt"), "ignored, not md").unwrap();

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        scan_commands_dir(
            &root,
            &root,
            "project",
            Some("my-proj"),
            None,
            &mut out,
            &mut seen,
        );

        let names: std::collections::HashSet<_> = out.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains("review"), "{names:?}");
        assert!(
            names.contains("git:commit"),
            "nested namespaced with ':': {names:?}"
        );
        assert!(!names.iter().any(|n| n.contains("notes")), "non-md ignored");
        let review = out.iter().find(|c| c.name == "review").unwrap();
        assert_eq!(review.description, "Review code");
        assert_eq!(review.title, "/review", "命令展示名带前导 /");
        assert_eq!(review.kind, "command");
        assert_eq!(review.origin, "project");
        assert_eq!(review.origin_name.as_deref(), Some("my-proj"));
        // frontmatter argument-hint 去引号后原样带出（选中命令后作 ghost 占位）。
        assert_eq!(
            review.argument_hint.as_deref(),
            Some("[--wait|--background] [--base <ref>]")
        );
        // 无 frontmatter 的命令没有 hint。
        let commit = out.iter().find(|c| c.name == "git:commit").unwrap();
        assert_eq!(commit.argument_hint, None);

        // 同名再扫一次（如用户级）不应重复加入。
        let before = out.len();
        scan_commands_dir(&root, &root, "user", None, None, &mut out, &mut seen);
        assert_eq!(out.len(), before, "已 seen 的名字不重复");

        // 插件命令带 `<plugin>:` 命名空间 —— 调用名 = `codex:review`、展示名 = `/codex:review`
        // （= CLI 认的真实命令，不能是裸 `/review`）；角标显示美化名，不参与命名空间。
        let mut pout = Vec::new();
        let mut pseen = std::collections::HashSet::new();
        scan_commands_dir(
            &root,
            &root,
            "plugin",
            Some("Codex"),
            Some("codex"),
            &mut pout,
            &mut pseen,
        );
        let pnames: std::collections::HashSet<_> = pout.iter().map(|c| c.name.as_str()).collect();
        assert!(
            pnames.contains("codex:review"),
            "插件命令需 codex: 前缀: {pnames:?}"
        );
        assert!(
            pnames.contains("codex:git:commit"),
            "嵌套插件命令: {pnames:?}"
        );
        assert!(!pnames.contains("review"), "插件命令不应是裸名");
        let preview = pout.iter().find(|c| c.name == "codex:review").unwrap();
        assert_eq!(preview.title, "/codex:review");
        assert_eq!(preview.origin_name.as_deref(), Some("Codex"));
    }

    #[test]
    fn scan_skills_dir_collects_all_with_prettified_title() {
        let root = std::env::temp_dir().join("csv-claude-skill-scan");
        let _ = fs::remove_dir_all(&root);
        // 一个带 frontmatter name + description，一个只有目录名、无 user-invocable。
        fs::create_dir_all(root.join("animejs")).unwrap();
        fs::write(
            root.join("animejs").join("SKILL.md"),
            "---\nname: animejs\ndescription: Anime.js adapter\n---\nbody",
        )
        .unwrap();
        fs::create_dir_all(root.join("create-promo-video")).unwrap();
        fs::write(
            root.join("create-promo-video").join("SKILL.md"),
            "no frontmatter body line",
        )
        .unwrap();
        // 没有 SKILL.md 的目录应被忽略。
        fs::create_dir_all(root.join("empty")).unwrap();

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        scan_skills_dir(&root, "user", None, None, &mut out, &mut seen);

        let anime = out.iter().find(|c| c.name == "animejs").unwrap();
        assert_eq!(anime.title, "Animejs");
        assert_eq!(anime.kind, "skill");
        assert_eq!(anime.description, "Anime.js adapter");
        assert_eq!(anime.origin, "user");
        assert!(anime.origin_name.is_none(), "user 来源不带 origin_name");
        assert_eq!(anime.argument_hint, None, "技能不带 argument-hint");
        // 无 frontmatter name → 回退目录名；展示名美化为 Title Case。
        let promo = out.iter().find(|c| c.name == "create-promo-video").unwrap();
        assert_eq!(promo.title, "Create Promo Video");
        assert!(
            !out.iter().any(|c| c.name == "empty"),
            "无 SKILL.md 的目录忽略"
        );

        // 插件技能同样带 `<plugin>:` 命名空间：调用名 = `codex:animejs`，展示名（title）仍是美化基础名。
        let mut pout = Vec::new();
        let mut pseen = std::collections::HashSet::new();
        scan_skills_dir(
            &root,
            "plugin",
            Some("Codex"),
            Some("codex"),
            &mut pout,
            &mut pseen,
        );
        let panime = pout.iter().find(|c| c.name == "codex:animejs").unwrap();
        assert_eq!(panime.title, "Animejs", "title 美化基础名，不含命名空间");
        assert_eq!(panime.origin_name.as_deref(), Some("Codex"));
        assert!(
            !pout.iter().any(|c| c.name == "animejs"),
            "插件技能不应是裸名"
        );
    }

    #[test]
    fn parse_frontmatter_unfolds_block_scalar_description() {
        let fm = parse_frontmatter(
            "---\nname: humanizer\ndescription: |\n  Remove signs of AI writing\n  from a draft.\nallowed-tools: \"Read\"\n---\nbody",
        );
        assert_eq!(fm.get("name").map(String::as_str), Some("humanizer"));
        assert_eq!(
            fm.get("description").map(String::as_str),
            Some("Remove signs of AI writing from a draft.")
        );
        // 块标量后顶格的下一个键照常解析。
        assert_eq!(fm.get("allowed-tools").map(String::as_str), Some("Read"));
    }

    #[test]
    fn prettify_name_title_cases_tokens() {
        assert_eq!(prettify_name("animejs"), "Animejs");
        assert_eq!(prettify_name("create-promo-video"), "Create Promo Video");
        assert_eq!(prettify_name("planning_with-files"), "Planning With Files");
    }
}
