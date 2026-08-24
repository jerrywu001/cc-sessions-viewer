// Codex 会话源：~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl
//
// Codex 的 JSONL 比 Claude 更"事件流"一些 —— 每行要么是 `event_msg`（高层对话事件，
// 文本干净）要么是 `response_item`（OpenAI ChatCompletion 原始 item，包含工具调用 /
// 多模态 content 数组）。我们用 event_msg 拿对话文本，用 response_item 抢救图片
// 和工具调用细节。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use serde_json::{json, Map};

use super::{ChatEvent, ChatProcessModel, SessionSource};
use crate::agent_command::AgentCommand;
use crate::stats::{
    pricing, shell as shell_util,
    types::{CallRecord, Turn},
};
use crate::types::{Block, Msg, ProjectInfo, SessionMeta, SessionPage, UsageSummary};
use crate::util::{
    append_jsonl_line, clean_title, format_iso8601_utc, home, inline_at_file_path, is_jsonl,
    mtime_millis, parse_iso8601_ms, simple_msg, strip_edge_references, text_block,
    validate_rename_name,
};

pub struct CodexSource;

const CODEX_APP_FIRST_PAGE_SIZE: usize = 50;
const CODEX_APP_LIST_PAGE_SIZE: usize = 100;
const CODEX_APP_LIST_MAX_THREADS: usize = 1_000;

fn sessions_dir() -> PathBuf {
    home().join(".codex").join("sessions")
}

fn archived_sessions_dir() -> PathBuf {
    home().join(".codex").join("archived_sessions")
}

/// 在 ~/.codex 下找编号最大的 state_<N>.sqlite —— codex 用版本号区分 schema，
/// 升级时会写到新文件（state_4.sqlite → state_5.sqlite），picker 用最新的那个。
/// 没找到时返回 None，调用方应静默跳过 sqlite 更新（codex 旧版本或从未运行）。
fn find_state_db() -> Option<PathBuf> {
    let dir = home().join(".codex");
    let mut best: Option<(u64, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let n = name
                .strip_prefix("state_")
                .and_then(|s| s.strip_suffix(".sqlite"))
                .and_then(|s| s.parse::<u64>().ok());
            if let Some(n) = n {
                if best.as_ref().map(|(b, _)| n > *b).unwrap_or(true) {
                    best = Some((n, e.path()));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

struct Meta {
    id: String,
    cwd: String,
    created: Option<String>,
}

#[derive(Debug, Clone)]
struct TitleIndexEntry {
    name: String,
    updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct CodexAppThreadInfo {
    rank: usize,
}

#[derive(Debug)]
struct CodexAppListSnapshot {
    available: bool,
    scanned: usize,
    first_page_size: usize,
    threads: HashMap<String, CodexAppThreadInfo>,
}

#[derive(Debug, Default, Clone, Copy)]
struct CodexThreadFlags {
    internal: bool,
    archived: bool,
}

#[derive(Debug, Default)]
struct CodexThreadFlagsIndex {
    by_id: HashMap<String, CodexThreadFlags>,
    by_path: HashMap<String, CodexThreadFlags>,
}

fn is_archived_path(path: &Path) -> bool {
    path.starts_with(archived_sessions_dir())
}

fn load_thread_flags_index() -> CodexThreadFlagsIndex {
    let Some(db_path) = find_state_db() else {
        return CodexThreadFlagsIndex::default();
    };
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(conn) => conn,
        Err(_) => return CodexThreadFlagsIndex::default(),
    };
    let mut stmt = match conn.prepare(
        "SELECT id, rollout_path, archived, has_user_event, source, thread_source, model FROM threads",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return CodexThreadFlagsIndex::default(),
    };
    let rows = match stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let rollout_path: String = row.get(1)?;
        let archived: i64 = row.get(2)?;
        let has_user_event: i64 = row.get(3)?;
        let source: String = row.get(4).unwrap_or_default();
        let thread_source: Option<String> = row.get(5).unwrap_or_default();
        let model: Option<String> = row.get(6).unwrap_or_default();
        let flags = thread_flags_from_fields(
            archived != 0,
            has_user_event,
            &source,
            thread_source.as_deref(),
            model.as_deref(),
        );
        Ok((id, rollout_path, flags))
    }) {
        Ok(rows) => rows,
        Err(_) => return CodexThreadFlagsIndex::default(),
    };
    let mut index = CodexThreadFlagsIndex::default();
    for row in rows.flatten() {
        let (id, rollout_path, flags) = row;
        index.by_path.insert(rollout_path, flags);
        index.by_id.insert(id, flags);
    }
    index
}

fn thread_flags_from_fields(
    archived: bool,
    _has_user_event: i64,
    source: &str,
    thread_source: Option<&str>,
    model: Option<&str>,
) -> CodexThreadFlags {
    let source_lc = source.to_lowercase();
    let thread_source_lc = thread_source.unwrap_or_default().to_lowercase();
    let model_lc = model.unwrap_or_default().to_lowercase();
    let internal = thread_source_lc == "subagent"
        || source_lc.contains("guardian")
        || model_lc == "codex-auto-review";
    CodexThreadFlags { internal, archived }
}

fn flags_for(fp: &Path, meta: &Meta, index: &CodexThreadFlagsIndex) -> CodexThreadFlags {
    let path = fp.to_string_lossy().to_string();
    let mut flags = index
        .by_id
        .get(&meta.id)
        .or_else(|| index.by_path.get(&path))
        .copied()
        .unwrap_or_default();
    if is_archived_path(fp) {
        flags.archived = true;
    }
    flags
}

fn include_by_flags(
    flags: CodexThreadFlags,
    include_internal: bool,
    include_archived: bool,
) -> bool {
    if flags.archived {
        return include_archived;
    }
    if flags.internal {
        return include_internal;
    }
    true
}

impl Default for CodexAppListSnapshot {
    fn default() -> Self {
        Self {
            available: false,
            scanned: 0,
            first_page_size: CODEX_APP_FIRST_PAGE_SIZE,
            threads: HashMap::new(),
        }
    }
}

/// 递归收集 Codex rollout JSONL。默认只扫 ~/.codex/sessions；
/// 用户显式打开“已归档会话”时再额外扫 ~/.codex/archived_sessions。
fn all_files(include_archived: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_jsonl(&sessions_dir(), &mut out);
    if include_archived {
        collect_jsonl(&archived_sessions_dir(), &mut out);
    }
    out
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_jsonl(&p, out);
            } else if is_jsonl(&p) {
                out.push(p);
            }
        }
    }
}

fn augmented_path() -> String {
    let current = env::var("PATH").unwrap_or_default();
    let additions = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ];
    let mut parts: Vec<String> = additions.iter().map(|value| value.to_string()).collect();
    parts.extend(
        current
            .split(':')
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    );
    parts.dedup();
    parts.join(":")
}

fn codex_cli_path() -> PathBuf {
    if let Ok(path) = env::var("CODEX_CLI") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return candidate;
        }
    }
    for candidate in [
        "/opt/homebrew/bin/codex",
        "/usr/local/bin/codex",
        "/usr/bin/codex",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return path;
        }
    }
    PathBuf::from("codex")
}

fn app_server_response(
    rx: &mpsc::Receiver<String>,
    id: i64,
    timeout: Duration,
) -> Result<Value, String> {
    loop {
        let line = rx
            .recv_timeout(timeout)
            .map_err(|_| format!("Timed out waiting for app-server response: {id}"))?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(format!("app-server error: {error}"));
        }
        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
    }
}

fn query_codex_app_thread_list() -> CodexAppListSnapshot {
    let result = (|| -> Result<CodexAppListSnapshot, String> {
        let mut child = crate::util::silent_command(codex_cli_path())
            .arg("app-server")
            .env("PATH", augmented_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to launch codex app-server: {e}"))?;

        let scan = (|| -> Result<CodexAppListSnapshot, String> {
            if let Some(stderr) = child.stderr.take() {
                thread::spawn(move || {
                    let mut reader = BufReader::new(stderr);
                    let mut sink = String::new();
                    let _ = reader.read_to_string(&mut sink);
                });
            }

            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| "app-server stdout not available".to_string())?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| "app-server stdin not available".to_string())?;

            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = tx.send(line);
                }
            });

            writeln!(
                stdin,
                "{}",
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "clientInfo": {
                            "name": "cc-sessions-viewer",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "capabilities": { "experimentalApi": true },
                    },
                })
            )
            .map_err(|e| format!("Failed to write initialize: {e}"))?;
            stdin
                .flush()
                .map_err(|e| format!("Failed to flush initialize: {e}"))?;
            let _ = app_server_response(&rx, 1, Duration::from_secs(5))?;

            let mut threads = HashMap::new();
            let mut cursor: Option<String> = None;
            let mut rank = 0usize;
            let mut request_id = 2i64;

            loop {
                let limit = if cursor.is_none() {
                    CODEX_APP_FIRST_PAGE_SIZE
                } else {
                    CODEX_APP_LIST_PAGE_SIZE
                };
                let mut params = Map::new();
                params.insert("limit".into(), json!(limit));
                params.insert("archived".into(), json!(false));
                params.insert("sortKey".into(), json!("updated_at"));
                params.insert("sortDirection".into(), json!("desc"));
                if let Some(cursor_value) = cursor.clone() {
                    params.insert("cursor".into(), json!(cursor_value));
                }

                writeln!(
                    stdin,
                    "{}",
                    json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "method": "thread/list",
                        "params": Value::Object(params),
                    })
                )
                .map_err(|e| format!("Failed to write thread/list: {e}"))?;
                stdin
                    .flush()
                    .map_err(|e| format!("Failed to flush thread/list: {e}"))?;
                let response = app_server_response(&rx, request_id, Duration::from_secs(8))?;
                request_id += 1;

                if let Some(data) = response.get("data").and_then(Value::as_array) {
                    for item in data {
                        if let Some(id) = item.get("id").and_then(Value::as_str) {
                            rank += 1;
                            threads
                                .entry(id.to_string())
                                .or_insert(CodexAppThreadInfo { rank });
                        }
                    }
                }

                cursor = response
                    .get("nextCursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if cursor.is_none() || rank >= CODEX_APP_LIST_MAX_THREADS {
                    break;
                }
            }

            Ok(CodexAppListSnapshot {
                available: true,
                scanned: rank,
                first_page_size: CODEX_APP_FIRST_PAGE_SIZE,
                threads,
            })
        })();
        let _ = child.kill();
        let _ = child.wait();
        scan
    })();

    result.unwrap_or_default()
}

fn apply_codex_app_list_snapshot(sessions: &mut [SessionMeta], snapshot: &CodexAppListSnapshot) {
    for session in sessions {
        session.codex_app_first_page_size = snapshot.first_page_size;
        if !snapshot.available {
            session.codex_app_list_rank = None;
            session.codex_app_list_scanned = 0;
            session.codex_app_first_page_position = 0;
            continue;
        }
        let info = snapshot.threads.get(&session.id);
        session.codex_app_list_scanned = snapshot.scanned;
        session.codex_app_list_rank = info.map(|item| item.rank);
        session.codex_app_first_page_position = info
            .filter(|item| item.rank <= snapshot.first_page_size)
            .map(|item| item.rank)
            .unwrap_or(0);
    }
}

/// 读取首行 session_meta，得到 id / cwd / 创建时间。
fn meta(path: &Path) -> Option<Meta> {
    let file = fs::File::open(path).ok()?;
    let mut first = String::new();
    BufReader::new(file).read_line(&mut first).ok()?;
    let v: Value = serde_json::from_str(first.trim()).ok()?;
    if v.get("type").and_then(|x| x.as_str()) != Some("session_meta") {
        return None;
    }
    let p = v.get("payload")?;
    Some(Meta {
        id: p
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        cwd: p
            .get("cwd")
            .and_then(|x| x.as_str())
            .unwrap_or("(未知目录)")
            .to_string(),
        created: p
            .get("timestamp")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
    })
}

/// 读取 `~/.codex/session_index.jsonl`，返回 thread_id → 最新 thread_name。
/// 文件不存在 / 不可读时返回空 map，调用方自动回落到旧的 JSONL 内联策略。
fn load_title_index() -> HashMap<String, TitleIndexEntry> {
    let mut map: HashMap<String, TitleIndexEntry> = HashMap::new();
    let path = home().join(".codex").join("session_index.jsonl");
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return map,
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = match v.get("id").and_then(|x| x.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        if let Some(name) = v.get("thread_name").and_then(|x| x.as_str()) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                // append-only：后写入的覆盖先写入的
                map.insert(
                    id,
                    TitleIndexEntry {
                        name: trimmed.to_string(),
                        updated_at_ms: v
                            .get("updated_at")
                            .and_then(|x| x.as_str())
                            .and_then(parse_iso8601_ms),
                    },
                );
            }
        }
    }
    map
}

/// 取首条用户输入作为标题（用于回收站展示）。
fn first_user_text(fp: &Path) -> String {
    if let Ok(file) = fs::File::open(fp) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if let Some(text) = codex_user_text(&v) {
                    let c = clean_codex_title(&text);
                    if !c.is_empty() {
                        return c;
                    }
                }
            }
        }
    }
    "(untitled session)".to_string()
}

/// Newer Codex rollouts keep the user message in `response_item.message` and
/// repeat the normalized version in `event_msg.item_completed.UserMessage`.
/// Text blocks are the only part relevant to a title or message counter;
/// image/file blocks must not turn into a title by themselves.
fn content_text(content: Option<&Value>) -> Option<String> {
    let parts: Vec<&str> = content?
        .as_array()?
        .iter()
        .filter_map(
            |block| match block.get("type").and_then(Value::as_str).unwrap_or("") {
                // UserMessage uses lowercase `text`; AgentMessage and the
                // response stream use `Text` / `output_text` respectively.
                "input_text" | "text" | "Text" | "output_text" => {
                    block.get("text").and_then(Value::as_str)
                }
                _ => None,
            },
        )
        .filter(|text| !text.trim().is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn codex_user_text(value: &Value) -> Option<String> {
    match (
        value.get("type").and_then(Value::as_str),
        value
            .get("payload")
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str),
    ) {
        (Some("event_msg"), Some("user_message")) => value
            .get("payload")
            .and_then(|payload| payload.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        (Some("event_msg"), Some("item_completed"))
            if value
                .get("payload")
                .and_then(|payload| payload.get("item"))
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str)
                == Some("UserMessage") =>
        {
            content_text(
                value
                    .get("payload")
                    .and_then(|payload| payload.get("item"))
                    .and_then(|item| item.get("content")),
            )
        }
        (Some("response_item"), Some("message"))
            if value
                .get("payload")
                .and_then(|payload| payload.get("role"))
                .and_then(Value::as_str)
                == Some("user") =>
        {
            content_text(
                value
                    .get("payload")
                    .and_then(|payload| payload.get("content")),
            )
        }
        _ => None,
    }
}

fn is_codex_internal_user_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("# AGENTS.md instructions")
        || trimmed.starts_with("<skills_instructions>")
        || trimmed.starts_with("<environment_context>")
        || trimmed.starts_with("<system-reminder>")
        || trimmed.starts_with("<turn_aborted>")
}

fn clean_codex_title(text: &str) -> String {
    if is_codex_internal_user_text(text) {
        return String::new();
    }
    let (_, body) = extract_codex_files(text);
    clean_title(&body)
}

/// Codex: `{"type":"input_image","image_url":"data:...|http..."}`
/// 兼容 `image_url` 为对象 `{"url":"..."}` 的旧/上游 OpenAI 格式。
fn image_src(el: &Value) -> Option<String> {
    if el.get("type").and_then(|x| x.as_str()) != Some("input_image") {
        return None;
    }
    let v = el.get("image_url")?;
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Object(_) => v
            .get("url")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn image_token_number(token: &str) -> Option<usize> {
    let token = token.strip_prefix('[')?.strip_suffix(']')?;
    let number = token
        .strip_prefix("Image #")
        .or_else(|| token.strip_prefix("#image "))?;
    number.parse::<usize>().ok().filter(|n| *n > 0)
}

fn image_tokens_in_message(msg: &Msg) -> Vec<String> {
    let Ok(token_re) = regex_lite::Regex::new(r"\[(?:Image #\d+|#image \d+)\]") else {
        return Vec::new();
    };
    msg.blocks
        .iter()
        .filter(|block| block.kind == "text")
        .filter_map(|block| block.text.as_deref())
        .flat_map(|text| token_re.find_iter(text).map(|m| m.as_str().to_string()))
        .collect()
}

/// `view_image` is recorded as a JavaScript tool input rather than a JSON path
/// field. Recover its path so the resulting base64 image can hydrate the
/// original user attachment, even after the temp file has been removed.
fn tool_input_path(input: &str) -> Option<String> {
    let marker = input.find("path")?;
    let rest = &input[marker + "path".len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    for ch in rest[quote.len_utf8()..].chars() {
        if escaped {
            value.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '\\' => '\\',
                '"' => '"',
                '\'' => '\'',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return (!value.is_empty()).then_some(value);
        } else {
            value.push(ch);
        }
    }
    None
}

fn hydrate_codex_image_attachment(msgs: &mut [Msg], path: &str, src: String) {
    for msg in msgs.iter_mut().rev() {
        let tokens = image_tokens_in_message(msg);
        let mut attachment_position = 0usize;
        for block in &mut msg.blocks {
            if block.kind != "image" && block.kind != "file" {
                continue;
            }
            attachment_position += 1;
            let matches = block.file_path.as_deref() == Some(path)
                || block.image_src.as_deref() == Some(path);
            if !matches {
                continue;
            }
            block.kind = "image".to_string();
            block.file_path = None;
            block.image_src = Some(src);
            if block.inline_placeholder.is_none() {
                block.inline_placeholder = tokens
                    .iter()
                    .find(|token| image_token_number(token) == Some(attachment_position))
                    .cloned();
            }
            return;
        }
    }
}

/// 从 Codex 原始 user content 中读取贴图与正文占位符的对应关系。
///
/// Codex 会把一张图拆成三个相邻的 block：打开 `<image name=[Image #N]>` 的
/// `input_text`、`input_image`、关闭 `</image>` 的 `input_text`。Read 模式不能
/// 只收集所有 `input_image`，否则多个图片/文件混排时会丢失原始附件位置。
fn codex_input_images(content: &Value) -> Vec<Block> {
    let Some(arr) = content.as_array() else {
        return Vec::new();
    };
    let token_re =
        regex_lite::Regex::new(r"\[(?:Image #\d+|#image \d+)\]").expect("valid image token regex");
    let mut current_placeholder: Option<String> = None;
    let mut images = Vec::new();

    for el in arr {
        match el.get("type").and_then(Value::as_str) {
            Some("input_text") => {
                let text = el.get("text").and_then(Value::as_str).unwrap_or_default();
                if text.contains("<image") {
                    current_placeholder = token_re.captures(text).and_then(|caps| {
                        caps.get(0).and_then(|token| {
                            image_token_number(token.as_str()).map(|n| {
                                if token.as_str().starts_with("[#image ") {
                                    format!("[#image {n}]")
                                } else {
                                    format!("[Image #{n}]")
                                }
                            })
                        })
                    });
                }
                if text.contains("</image>") {
                    current_placeholder = None;
                }
            }
            Some("input_image") => {
                if let Some(src) = image_src(el) {
                    images.push(Block {
                        kind: "image".to_string(),
                        image_src: Some(src),
                        inline_placeholder: current_placeholder.clone(),
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }
    images
}

/// 用 Codex 写入正文的 `[Image #N]` 还原所有附件的原始顺序。
/// N 不是图片数组下标，而是当前消息中 file/image 附件的 1-based 位置。
fn order_codex_attachments(images: Vec<Block>, files: Vec<Block>) -> Vec<Block> {
    if images.is_empty() {
        return files;
    }
    if !images
        .iter()
        .any(|block| block.inline_placeholder.is_some())
    {
        let mut blocks = images;
        blocks.extend(files);
        return blocks;
    }

    let total = images.len() + files.len();
    let max_position = images
        .iter()
        .filter_map(|block| block.inline_placeholder.as_deref())
        .filter_map(image_token_number)
        .max()
        .unwrap_or(0);
    let slot_count = total.max(max_position);
    let mut slots: Vec<Option<Block>> = (0..slot_count).map(|_| None).collect();
    let mut unpositioned_images = Vec::new();
    for image in images {
        let position = image
            .inline_placeholder
            .as_deref()
            .and_then(image_token_number);
        if let Some(position) = position {
            if position > 0 && position <= slots.len() && slots[position - 1].is_none() {
                slots[position - 1] = Some(image);
                continue;
            }
        }
        unpositioned_images.push(image);
    }

    let mut files = files.into_iter();
    let mut unpositioned_images = unpositioned_images.into_iter();
    let mut ordered = Vec::with_capacity(slot_count);
    for slot in slots {
        if let Some(block) = slot {
            ordered.push(block);
        } else if let Some(file) = files.next() {
            ordered.push(file);
        } else if let Some(image) = unpositioned_images.next() {
            ordered.push(image);
        }
    }
    ordered.extend(files);
    ordered.extend(unpositioned_images);
    ordered
}

fn format_args(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(parsed) => serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| s.clone()),
            Err(_) => s.clone(),
        },
        Some(other) => serde_json::to_string_pretty(other).unwrap_or_default(),
        None => String::new(),
    }
}

fn output_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn text_indicates_tool_error(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("script error")
        || text.contains("verification failed")
        || text.contains("patch failed")
}

fn output_indicates_tool_error(output: Option<&Value>) -> bool {
    match output {
        Some(Value::String(text)) => text_indicates_tool_error(text),
        Some(Value::Array(items)) => items.iter().any(|item| match item {
            Value::String(text) => text_indicates_tool_error(text),
            Value::Object(_) => item
                .get("text")
                .and_then(Value::as_str)
                .map(text_indicates_tool_error)
                .unwrap_or(false),
            _ => false,
        }),
        _ => false,
    }
}

/// Newer Codex CLI versions wrap `apply_patch` in the JavaScript `exec` tool:
///
/// ```text
/// const patch = "*** Begin Patch\\n...";
/// const result = await tools.apply_patch(patch);
/// ```
///
/// Restore the underlying patch only for this exact tool invocation. Other `exec`
/// calls must remain ordinary tool calls.
fn extract_exec_apply_patch(input: &str) -> Option<String> {
    let call_pos = input.find("tools.apply_patch")?;
    let mut rest = input[call_pos + "tools.apply_patch".len()..].trim_start();
    rest = rest.strip_prefix('(')?.trim_start();

    if rest.starts_with('"') {
        return decode_js_double_quoted_string(rest);
    }

    let ident_len = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$')
        .count();
    if ident_len == 0 {
        return None;
    }
    let variable = &rest[..ident_len];
    let call_tail = rest[ident_len..].trim_start();
    if !call_tail.starts_with(')') {
        return None;
    }

    let input_bytes = input.as_bytes();
    for declaration in ["const", "let", "var"] {
        let marker = format!("{declaration} {variable}");
        let mut offset = 0;
        while let Some(found) = input[offset..].find(&marker) {
            let start = offset + found;
            let before = input_bytes[..start].last().copied();
            let after = input_bytes[start + marker.len()..].first().copied();
            offset = start + marker.len();
            if before.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || after.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                continue;
            }
            let value = input[offset..].trim_start();
            let value = value.strip_prefix('=')?.trim_start();
            if let Some(patch) = decode_js_double_quoted_string(value) {
                return Some(patch);
            }
        }
    }
    None
}

/// The wrapper serializes `patch` as a JSON-compatible JavaScript string. Slice
/// precisely one literal before decoding so source following it is never evaluated.
fn decode_js_double_quoted_string(source: &str) -> Option<String> {
    if !source.starts_with('"') {
        return None;
    }
    let bytes = source.as_bytes();
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return serde_json::from_str(&source[..=index]).ok(),
            _ => index += 1,
        }
    }
    None
}

fn apply_patch_section_order(input: &str) -> Vec<String> {
    let mut order = Vec::new();
    for line in input.lines() {
        for prefix in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
            if let Some(path) = line.strip_prefix(prefix) {
                order.push(path.to_string());
                break;
            }
        }
    }
    order
}

fn build_apply_patch_section(path: &str, change: &Value) -> Option<String> {
    let op = change.get("type").and_then(Value::as_str)?;
    let move_path = change.get("move_path").and_then(Value::as_str);
    let mut lines = Vec::new();
    match op {
        "update" => {
            lines.push(format!("*** Update File: {path}"));
            if let Some(target) = move_path.filter(|target| !target.is_empty() && *target != path) {
                lines.push(format!("*** Move to: {target}"));
            }
            if let Some(diff) = change.get("unified_diff").and_then(Value::as_str) {
                lines.extend(diff.lines().map(str::to_owned));
            }
        }
        "add" => {
            let target = move_path
                .filter(|target| !target.is_empty())
                .unwrap_or(path);
            lines.push(format!("*** Add File: {target}"));
            if let Some(content) = change.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    lines.push(format!("@@ -0,0 +1,{} @@", content.lines().count()));
                    lines.extend(content.lines().map(|line| format!("+{line}")));
                }
            }
        }
        "delete" => {
            lines.push(format!("*** Delete File: {path}"));
            if let Some(content) = change.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    lines.push(format!("@@ -1,{} +0,0 @@", content.lines().count()));
                    lines.extend(content.lines().map(|line| format!("-{line}")));
                }
            }
        }
        _ => return None,
    }
    Some(lines.join("\n"))
}

fn augment_apply_patch_input(input: &str, changes: &Value) -> Option<String> {
    let changes = changes.as_object()?;
    if changes.is_empty() {
        return None;
    }

    let mut sections = Vec::new();
    let mut used_paths: HashMap<String, bool> = HashMap::new();
    for path in apply_patch_section_order(input) {
        if used_paths.contains_key(&path) {
            continue;
        }
        let Some(change) = changes.get(&path) else {
            continue;
        };
        let Some(section) = build_apply_patch_section(&path, change) else {
            continue;
        };
        sections.push(section);
        used_paths.insert(path, true);
    }
    for (path, change) in changes {
        if used_paths.contains_key(path) {
            continue;
        }
        let Some(section) = build_apply_patch_section(path, change) else {
            continue;
        };
        sections.push(section);
    }
    if sections.is_empty() {
        return None;
    }
    let mut patch = String::from("*** Begin Patch\n");
    patch.push_str(&sections.join("\n"));
    patch.push_str("\n*** End Patch");
    Some(patch)
}

/// `patch_apply_end` emitted by the JavaScript `exec` wrapper has an internal
/// `exec-*` ID, not the outer custom-tool call ID. Match the event back to the
/// newest outstanding wrapper that requested every changed path.
fn patch_changes_match_input(input: &str, changes: &Value) -> bool {
    let Some(changes) = changes.as_object() else {
        return false;
    };
    if changes.is_empty() {
        return false;
    }
    let paths = apply_patch_section_order(input);
    changes
        .keys()
        .all(|path| paths.iter().any(|known| known == path))
}

fn agent_message_phase(payload: &Value) -> Option<&str> {
    payload.get("phase").and_then(Value::as_str)
}

/// Codex Desktop 用户带文件提问时，message 文本是一段固定结构（见 rollout JSONL 原文）：
///
/// ```text
/// # Files mentioned by the user:
///
/// ## devtools_options.yaml: /abs/path/devtools_options.yaml
///
/// ## My request for Codex:
/// hi
/// ```
///
/// 图片已从 response_item 捕获时，提示词首尾连续的 `@"path"` / `@path` 附件引用可提前
/// 移除；中间引用必须完整保留给下方提示词，公共后处理会为它补文件 tag 并负责去重。
#[allow(dead_code)]
fn strip_edge_at_paths(text: &str) -> String {
    let re = regex_lite::Regex::new(r#"@"[^"]+"|@\S+"#).expect("valid regex");
    let mut refs = Vec::new();
    for whole in re.find_iter(text) {
        let raw = &text[whole.start() + 1..whole.end()];
        let reference_end = if raw
            .strip_prefix('"')
            .and_then(|quoted| quoted.strip_suffix('"'))
            .is_some_and(|quoted| !quoted.is_empty())
        {
            Some(whole.end())
        } else {
            inline_at_file_path(raw, None).map(|path| whole.start() + 1 + path.len())
        };
        if let Some(reference_end) = reference_end {
            refs.push((whole.start(), reference_end));
        }
    }
    strip_edge_references(text, &refs).trim().to_string()
}

/// 把每个 `## <name>: <path>` 抽成 `file` 块（点击外部打开），正文换成 `## My request for
/// Codex:` 之后的真实请求。没有这个结构就原样返回（文件块为空）。
pub fn extract_codex_files_pub(text: &str) -> (Vec<Block>, String) {
    extract_codex_files(text)
}

/// GUI 粘贴板图片会先保存到应用自己的临时图片目录，再由 Codex 以普通文件
/// mention 写入 rollout。旧格式没有保留 `inlinePlaceholder`，但这个路径仍能
/// 区分粘贴图和用户选择的普通图片附件。
fn is_codex_pasted_image_path(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component.as_os_str() == "cc-sessions-viewer-images")
}

/// 为旧 Codex rollout 中的粘贴图恢复正文 `[Image #N]` 标签。
fn bind_codex_pasted_file_placeholders(body: &str, blocks: &mut [Block]) {
    let token_re = regex_lite::Regex::new(r"\[Image #\d+\]").expect("valid image token regex");
    let tokens: Vec<String> = token_re
        .find_iter(body)
        .map(|m| m.as_str().to_string())
        .collect();
    let mut next = 0usize;
    for block in blocks {
        if block.kind != "image" || block.inline_placeholder.is_some() {
            continue;
        }
        let Some(path) = block.image_src.as_deref() else {
            continue;
        };
        if !is_codex_pasted_image_path(path) {
            continue;
        }
        if let Some(token) = tokens.get(next) {
            block.inline_placeholder = Some(token.clone());
            next += 1;
        }
    }
}

fn extract_codex_files(text: &str) -> (Vec<Block>, String) {
    const HEADER: &str = "# Files mentioned by the user:";
    const REQUEST: &str = "## My request for Codex:";
    let (Some(hidx), Some(ridx)) = (text.find(HEADER), text.find(REQUEST)) else {
        return (Vec::new(), text.to_string());
    };
    if ridx < hidx {
        return (Vec::new(), text.to_string());
    }
    let mut files = Vec::new();
    for line in text[hidx + HEADER.len()..ridx].lines() {
        let Some(rest) = line.trim().strip_prefix("## ") else {
            continue;
        };
        // `<name>: <path>` —— 取第一个 `: ` 之后的整段当 path（name 即 basename，一般不含冒号）。
        if let Some((_, path)) = rest.split_once(": ") {
            let path = path.trim();
            if !path.is_empty() {
                let pb = std::path::Path::new(path);
                if pb.exists() && crate::util::is_image_file(pb) {
                    files.push(Block {
                        kind: "image".to_string(),
                        image_src: Some(path.to_string()),
                        ..Default::default()
                    });
                } else {
                    files.push(Block {
                        kind: "file".to_string(),
                        file_path: Some(path.to_string()),
                        ..Default::default()
                    });
                }
            }
        }
    }
    if files.is_empty() {
        return (Vec::new(), text.to_string());
    }
    (files, text[ridx + REQUEST.len()..].trim().to_string())
}

fn user_message_has_content(payload: &Value) -> bool {
    payload
        .get("message")
        .and_then(Value::as_str)
        .is_some_and(|message| !message.trim().is_empty())
        || ["images", "local_images", "text_elements"]
            .iter()
            .any(|key| {
                payload
                    .get(key)
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty())
            })
}

/// Classify a Codex JSONL entry for turn-state inference.
/// Returns "started" / "completed" / "failed" / None.
pub fn classify_turn_state(value: &Value) -> Option<&'static str> {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = value.get("payload")?;
    match payload.get("type").and_then(Value::as_str)? {
        "user_message" if user_message_has_content(payload) => Some("started"),
        "task_complete" => Some("completed"),
        "agent_message" => {
            if agent_message_phase(payload) == Some("commentary") {
                None
            } else {
                Some("completed")
            }
        }
        "task_failed" | "error" => Some("failed"),
        _ => None,
    }
}

fn rename_system_reminder(name: &str) -> String {
    format!(
        "<system-reminder>\nThe user named this session \"{name}\". This may indicate the session's focus or intent.\n</system-reminder>"
    )
}

fn rename_system_msg(ts: Option<String>, name: &str) -> Msg {
    simple_msg(
        "user",
        ts,
        text_block("text", &rename_system_reminder(name)),
    )
}

fn should_synthesize_title_rename(
    title_name: &str,
    first_user_title: &str,
    created_ms: Option<i64>,
    updated_at_ms: Option<i64>,
) -> bool {
    if title_name.trim().is_empty() {
        return false;
    }
    if !first_user_title.trim().is_empty() && title_name.trim() == first_user_title.trim() {
        return false;
    }
    let (Some(created_ms), Some(updated_at_ms)) = (created_ms, updated_at_ms) else {
        return false;
    };
    // `session_index.jsonl` 既有首次自动命名，也有用户中途 rename。
    // 只把明显晚于建会话时间的更新当成 rename 事件，避免给每条会话都凭空加一行。
    updated_at_ms.saturating_sub(created_ms) > 60_000
}

fn format_iso8601ish(ms: i64) -> String {
    format_iso8601_utc(ms.div_euclid(1000), ms.rem_euclid(1000) as u32)
}

fn scan(
    fp: &Path,
    m: &Meta,
    title_index: &HashMap<String, TitleIndexEntry>,
    flags: CodexThreadFlags,
) -> SessionMeta {
    let file_name = fp
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let size = fs::metadata(fp).map(|m| m.len()).unwrap_or(0);
    let modified = mtime_millis(fp);

    // Codex rename 会追加 `event_msg.payload.type == "thread_name_updated"`，
    // 最后一条 `thread_name` 生效。优先用它，没有则回落首条 user_message。
    let mut first_user_title = String::new();
    let mut thread_name: Option<String> = None;
    let mut old_user_message_count = 0usize;
    let mut old_agent_message_count = 0usize;
    let mut completed_user_message_count = 0usize;
    let mut completed_agent_message_count = 0usize;
    let mut completed_user_message_ids = HashSet::new();
    let mut response_user_messages: Vec<String> = Vec::new();
    if let Ok(file) = fs::File::open(fp) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let p = match v.get("payload") {
                Some(p) => p,
                None => continue,
            };
            let pt = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if pt == "thread_name_updated" {
                if let Some(name) = p.get("thread_name").and_then(|x| x.as_str()) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() {
                        thread_name = Some(trimmed.to_string());
                    }
                }
                continue;
            }
            if pt == "user_message" {
                old_user_message_count += 1;
                if first_user_title.is_empty() {
                    if let Some(msg) = codex_user_text(&v) {
                        let clean = clean_codex_title(&msg);
                        if !clean.is_empty() {
                            first_user_title = clean;
                        }
                    }
                }
                continue;
            }
            if pt == "agent_message" && agent_message_phase(p) != Some("commentary") {
                old_agent_message_count += 1;
                continue;
            }
            if pt == "item_completed"
                && p.get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("UserMessage")
            {
                let item = p.get("item").unwrap_or(&Value::Null);
                let item_id = item.get("id").and_then(Value::as_str);
                if item_id.is_none_or(|id| completed_user_message_ids.insert(id.to_string())) {
                    completed_user_message_count += 1;
                }
                if first_user_title.is_empty() {
                    if let Some(msg) = codex_user_text(&v) {
                        let clean = clean_codex_title(&msg);
                        if !clean.is_empty() {
                            first_user_title = clean;
                        }
                    }
                }
                continue;
            }
            if pt == "item_completed"
                && p.get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("AgentMessage")
                && p.get("item")
                    .and_then(|item| item.get("phase"))
                    .and_then(Value::as_str)
                    != Some("commentary")
            {
                completed_agent_message_count += 1;
                continue;
            }
            if v.get("type").and_then(Value::as_str) == Some("response_item")
                && pt == "message"
                && p.get("role").and_then(Value::as_str) == Some("user")
            {
                // 新格式会在后面追加 item_completed.UserMessage；先缓存 response_item
                // 作为“文件尚未写完”的兜底，避免同一条消息在完整 rollout 中计数两次。
                if let Some(msg) = codex_user_text(&v) {
                    response_user_messages.push(msg);
                }
            }
        }
    }
    let (message_count, fallback_title) = if completed_user_message_count > 0 {
        (
            completed_user_message_count + completed_agent_message_count,
            first_user_title.clone(),
        )
    } else if old_user_message_count > 0 {
        (
            old_user_message_count + old_agent_message_count,
            first_user_title.clone(),
        )
    } else {
        let title = response_user_messages
            .iter()
            .map(|text| clean_codex_title(text))
            .find(|text| !text.is_empty())
            .unwrap_or_default();
        (response_user_messages.len(), title)
    };
    if first_user_title.is_empty() {
        first_user_title = fallback_title;
    }
    let id = if m.id.is_empty() {
        file_name.trim_end_matches(".jsonl").to_string()
    } else {
        m.id.clone()
    };
    // 标题优先级：session_index.jsonl（codex 自带 rename 的权威来源） >
    // rollout 内 thread_name_updated（旧版 app 的写入或 codex 在会话运行时的事件）
    // > 首条 user_message。
    let title = title_index
        .get(&id)
        .map(|entry| entry.name.clone())
        .or(thread_name)
        .unwrap_or_else(|| {
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
        cwd: Some(m.cwd.clone()),
        created: m.created.clone(),
        modified,
        size,
        message_count,
        pi_branch_count: None,
        pi_entry_count: None,
        codex_app_list_rank: None,
        codex_app_list_scanned: 0,
        codex_app_first_page_size: 50,
        codex_app_first_page_position: 0,
        codex_internal: flags.internal,
        codex_archived: flags.archived,
    }
}

/// 解析 Codex rollout：用 event_msg 取干净的对话文本，用 response_item 取工具调用 / 图片。
///
/// 图片处理：Codex 把贴图的 user message 同时写两条：
///   1. `response_item.message` (role=user)，content 数组里夹着 `input_image`
///      （真正的 base64 / URL 在这里）；
///   2. 紧接着一条 `event_msg.user_message`，message 字段是去掉图片占位
///      （`<image name=[Image #N]>...</image>`）之后的纯文本（用户键入的部分）。
///
/// 用 event_msg 那条作为最终用户气泡的文本来源，扫到对应 response_item 时
/// 先把里面的 `input_image` 块缓存起来，等到下一条 user_message 出现时一起渲染。
fn read_with_title_index(
    path: &str,
    title_index: &HashMap<String, TitleIndexEntry>,
) -> Result<Vec<Msg>, String> {
    let file = fs::File::open(path).map_err(|e| format!("Failed to open session: {e}"))?;
    let mut msgs = Vec::new();
    let mut pending_user_images: Vec<Block> = Vec::new();
    let mut tool_input_paths: HashMap<String, String> = HashMap::new();
    let mut apply_patch_by_call_id: HashMap<String, usize> = HashMap::new();
    let mut wrapped_apply_patch_indices: Vec<usize> = Vec::new();
    let mut session_id: Option<String> = None;
    let mut created_ms: Option<i64> = None;
    let mut first_user_title = String::new();
    let mut saw_explicit_rename = false;
    let mut model_hint: Option<String> = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let ts = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let p = match v.get("payload") {
            Some(p) => p,
            None => continue,
        };
        let pt = p.get("type").and_then(|x| x.as_str()).unwrap_or("");

        match (t, pt) {
            ("session_meta", _) => {
                if session_id.is_none() {
                    session_id = p.get("id").and_then(|x| x.as_str()).map(|s| s.to_string());
                }
                if created_ms.is_none() {
                    created_ms = p
                        .get("timestamp")
                        .and_then(|x| x.as_str())
                        .and_then(parse_iso8601_ms)
                        .or_else(|| ts.as_deref().and_then(parse_iso8601_ms));
                }
                if model_hint.is_none() {
                    if let Some(m) = p.get("model").and_then(|x| x.as_str()) {
                        if !m.is_empty() {
                            model_hint = Some(m.to_string());
                        }
                    }
                }
            }
            ("turn_context", _) => {
                if let Some(m) = p.get("model").and_then(|x| x.as_str()) {
                    if !m.is_empty() {
                        model_hint = Some(m.to_string());
                    }
                }
            }
            ("response_item", "message")
                if p.get("role").and_then(|x| x.as_str()) == Some("user") =>
            {
                // 不渲染整条 response_item.message —— 它还包含 <environment_context>
                // 等内部包裹，由 event_msg.user_message 负责干净文本。这里只抢救图片。
                pending_user_images
                    .extend(codex_input_images(p.get("content").unwrap_or(&Value::Null)));
            }
            ("event_msg", "user_message") => {
                let text = p.get("message").and_then(|x| x.as_str()).unwrap_or("");
                // `turn/steer` 的延后约束是模型控制面，不属于用户气泡；先剥离它，才能
                // 继续正确识别其内的 Codex 文件头。
                let visible_text = crate::util::visible_deferred_follow_up(text);
                let (mut file_blocks, body) = extract_codex_files(visible_text);
                if pending_user_images.is_empty() {
                    bind_codex_pasted_file_placeholders(&body, &mut file_blocks);
                }
                let blocks =
                    order_codex_attachments(std::mem::take(&mut pending_user_images), file_blocks);
                // 图片已由 response_item 捕获到 pending_user_images。只去除首尾的附件
                // 标记；行中文件引用仍要完整显示，post_process 会做附件 tag 去重。
                // `@文件` 选择结果属于正文语义，必须保留在原位置。只有旧版
                // 图片协议产生的首尾路径标记才需要清理；普通行内引用不能移位。
                if first_user_title.is_empty() {
                    let clean = clean_title(&body);
                    if !clean.is_empty() {
                        first_user_title = clean;
                    }
                }
                if !body.trim().is_empty() {
                    let mut blocks = blocks;
                    blocks.push(text_block("text", &body));
                    if !blocks.is_empty() {
                        msgs.push(Msg {
                            uuid: None,
                            role: "user".to_string(),
                            timestamp: ts,
                            model: None,
                            sidechain: false,
                            blocks,
                            meta_kind: None,
                        });
                    }
                    continue;
                }
                if !blocks.is_empty() {
                    msgs.push(Msg {
                        uuid: None,
                        role: "user".to_string(),
                        timestamp: ts,
                        model: None,
                        sidechain: false,
                        blocks,
                        meta_kind: None,
                    });
                }
            }
            ("event_msg", "agent_message") => {
                if let Some(m) = p.get("message").and_then(|x| x.as_str()) {
                    if !m.trim().is_empty() {
                        let blocks = split_thinking_blocks(m);
                        if !blocks.is_empty() {
                            msgs.push(Msg {
                                uuid: None,
                                role: "assistant".to_string(),
                                timestamp: ts,
                                model: model_hint.clone(),
                                sidechain: false,
                                blocks,
                                meta_kind: None,
                            });
                        }
                    }
                }
            }
            ("event_msg", "item_completed") => {
                let Some(item) = p.get("item") else {
                    continue;
                };
                if item.get("type").and_then(Value::as_str) == Some("UserMessage") {
                    let text = content_text(item.get("content"));
                    let visible_text = text
                        .as_deref()
                        .map(crate::util::visible_deferred_follow_up)
                        .unwrap_or_default();
                    let (mut file_blocks, body) = extract_codex_files(visible_text);
                    if pending_user_images.is_empty() {
                        bind_codex_pasted_file_placeholders(&body, &mut file_blocks);
                    }
                    let mut blocks = order_codex_attachments(
                        std::mem::take(&mut pending_user_images),
                        file_blocks,
                    );
                    if !body.trim().is_empty() {
                        blocks.push(text_block("text", &body));
                    }
                    if !blocks.is_empty() {
                        msgs.push(Msg {
                            uuid: item.get("id").and_then(Value::as_str).map(str::to_owned),
                            role: "user".to_string(),
                            timestamp: ts,
                            model: None,
                            sidechain: false,
                            blocks,
                            meta_kind: None,
                        });
                    }
                    continue;
                }
                if item.get("type").and_then(Value::as_str) == Some("AgentMessage") {
                    let Some(text) = content_text(item.get("content")) else {
                        continue;
                    };
                    let blocks = split_thinking_blocks(&text);
                    if !blocks.is_empty() {
                        msgs.push(Msg {
                            uuid: item.get("id").and_then(Value::as_str).map(str::to_owned),
                            role: "assistant".to_string(),
                            timestamp: ts,
                            model: model_hint.clone(),
                            sidechain: false,
                            blocks,
                            meta_kind: None,
                        });
                    }
                    continue;
                }
                if !matches!(
                    item.get("type").and_then(Value::as_str),
                    Some("Plan" | "plan")
                ) {
                    continue;
                }
                let Some(text) = item
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                else {
                    continue;
                };
                msgs.push(Msg {
                    uuid: None,
                    role: "assistant".to_string(),
                    timestamp: ts,
                    model: model_hint.clone(),
                    sidechain: false,
                    blocks: vec![text_block("text", text)],
                    meta_kind: None,
                });
            }
            ("event_msg", "thread_name_updated") => {
                if let Some(name) = p.get("thread_name").and_then(|x| x.as_str()) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() {
                        saw_explicit_rename = true;
                        msgs.push(rename_system_msg(ts, trimmed));
                    }
                }
            }
            ("response_item", "function_call") | ("response_item", "custom_tool_call") => {
                let raw_input = format_args(p.get("arguments").or_else(|| p.get("input")));
                let call_id = p.get("call_id").and_then(|x| x.as_str()).map(str::to_owned);
                if let (Some(call_id), Some(path)) =
                    (call_id.as_deref(), tool_input_path(&raw_input))
                {
                    tool_input_paths.insert(call_id.to_string(), path);
                }
                let mut name = p
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let mut input = raw_input;
                let mut wrapped_apply_patch = false;
                if name == "exec" {
                    if let Some(patch) = extract_exec_apply_patch(&input) {
                        name = "apply_patch".to_string();
                        input = patch;
                        wrapped_apply_patch = true;
                    }
                }
                let is_apply_patch = name == "apply_patch";
                let id = p
                    .get("call_id")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                let id_for_index = id.clone();
                let mut msg = simple_msg(
                    "assistant",
                    ts,
                    Block {
                        kind: "tool_use".to_string(),
                        tool_name: Some(name),
                        tool_input: Some(input),
                        tool_id: id,
                        ..Default::default()
                    },
                );
                msg.model = model_hint.clone();
                msgs.push(msg);
                if is_apply_patch {
                    let msg_index = msgs.len().saturating_sub(1);
                    if wrapped_apply_patch {
                        wrapped_apply_patch_indices.push(msg_index);
                    }
                    if let Some(call_id) = id_for_index {
                        apply_patch_by_call_id.insert(call_id, msg_index);
                    }
                }
            }
            ("response_item", "function_call_output")
            | ("response_item", "custom_tool_call_output") => {
                let id = p
                    .get("call_id")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                let image_path = id
                    .as_deref()
                    .and_then(|call_id| tool_input_paths.remove(call_id));
                let output_is_error = output_indicates_tool_error(p.get("output"));
                if output_is_error {
                    if let Some(msg_index) = id
                        .as_deref()
                        .and_then(|call_id| apply_patch_by_call_id.get(call_id))
                    {
                        if let Some(block) = msgs
                            .get_mut(*msg_index)
                            .and_then(|msg| msg.blocks.get_mut(0))
                        {
                            block.is_error = true;
                        }
                    }
                }
                let mut blocks = Vec::new();
                if let Some(arr) = p.get("output").and_then(|x| x.as_array()) {
                    for el in arr {
                        if let Some(src) = image_src(el) {
                            blocks.push(Block {
                                kind: "image".to_string(),
                                image_src: Some(src),
                                tool_id: id.clone(),
                                ..Default::default()
                            });
                        } else {
                            let text = match el {
                                Value::String(s) => s.clone(),
                                Value::Object(_) => el
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                                    .unwrap_or_else(|| el.to_string()),
                                other => other.to_string(),
                            };
                            if !text.trim().is_empty() {
                                let is_error = text_indicates_tool_error(&text);
                                blocks.push(Block {
                                    kind: "tool_result".to_string(),
                                    text: Some(text),
                                    tool_id: id.clone(),
                                    is_error,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                } else {
                    let out = output_text(p.get("output"));
                    if !out.trim().is_empty() {
                        blocks.push(Block {
                            kind: "tool_result".to_string(),
                            text: Some(out),
                            tool_id: id,
                            is_error: output_is_error,
                            ..Default::default()
                        });
                    }
                }
                if let (Some(path), Some(src)) = (
                    image_path,
                    blocks.iter().find_map(|block| {
                        (block.kind == "image")
                            .then(|| block.image_src.clone())
                            .flatten()
                    }),
                ) {
                    hydrate_codex_image_attachment(&mut msgs, &path, src);
                }
                if !blocks.is_empty() {
                    msgs.push(Msg {
                        uuid: None,
                        role: "user".to_string(),
                        timestamp: ts,
                        model: None,
                        sidechain: false,
                        blocks,
                        meta_kind: None,
                    });
                }
            }
            ("event_msg", "patch_apply_end") => {
                let Some(call_id) = p.get("call_id").and_then(|x| x.as_str()) else {
                    continue;
                };
                let changes = p.get("changes").unwrap_or(&Value::Null);
                let msg_index = apply_patch_by_call_id.get(call_id).copied().or_else(|| {
                    wrapped_apply_patch_indices
                        .iter()
                        .rposition(|msg_index| {
                            msgs.get(*msg_index)
                                .and_then(|msg| msg.blocks.first())
                                .and_then(|block| block.tool_input.as_deref())
                                .map(|input| patch_changes_match_input(input, changes))
                                .unwrap_or(false)
                        })
                        .map(|position| wrapped_apply_patch_indices.remove(position))
                });
                let Some(msg_index) = msg_index else {
                    continue;
                };
                let Some(block) = msgs
                    .get_mut(msg_index)
                    .and_then(|msg| msg.blocks.get_mut(0))
                else {
                    continue;
                };
                let original = block.tool_input.clone().unwrap_or_default();
                if let Some(next_input) = augment_apply_patch_input(&original, changes) {
                    block.tool_input = Some(next_input);
                }
            }
            ("response_item", "web_search_call") => {
                let query = p
                    .get("action")
                    .and_then(|a| a.get("query"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut msg = simple_msg(
                    "assistant",
                    ts,
                    Block {
                        kind: "tool_use".to_string(),
                        tool_name: Some("web_search".to_string()),
                        tool_input: Some(query),
                        ..Default::default()
                    },
                );
                msg.model = model_hint.clone();
                msgs.push(msg);
            }
            _ => {}
        }
    }
    if !saw_explicit_rename {
        if let Some(session_id) = session_id.as_deref() {
            if let Some(entry) = title_index.get(session_id) {
                if should_synthesize_title_rename(
                    &entry.name,
                    &first_user_title,
                    created_ms,
                    entry.updated_at_ms,
                ) {
                    let rename_msg =
                        rename_system_msg(entry.updated_at_ms.map(format_iso8601ish), &entry.name);
                    let insert_at = msgs
                        .iter()
                        .position(|m| {
                            m.timestamp
                                .as_deref()
                                .and_then(parse_iso8601_ms)
                                .zip(entry.updated_at_ms)
                                .map(|(msg_ms, rename_ms)| msg_ms > rename_ms)
                                .unwrap_or(false)
                        })
                        .unwrap_or(msgs.len());
                    msgs.insert(insert_at, rename_msg);
                }
            }
        }
    }
    // 兜底：若文件结尾仍有未消费的图片（异常截断），别把它们丢掉。
    if !pending_user_images.is_empty() {
        msgs.push(Msg {
            uuid: None,
            role: "user".to_string(),
            timestamp: None,
            model: None,
            sidechain: false,
            blocks: std::mem::take(&mut pending_user_images),
            meta_kind: None,
        });
    }
    Ok(msgs)
}

fn read(path: &str) -> Result<Vec<Msg>, String> {
    let title_index = load_title_index();
    read_with_title_index(path, &title_index)
}

fn post_process_codex_session_msgs(msgs: &mut [Msg]) {
    for msg in msgs {
        let skip_path_lifting = cfg!(windows)
            && msg
                .blocks
                .iter()
                .any(|block| block.kind == "image" && block.inline_placeholder.is_some());
        if skip_path_lifting {
            crate::util::post_process_session_msgs_without_path_lifting(std::slice::from_mut(msg));
        } else {
            crate::util::post_process_session_msgs_without_unmarked_path_lifting(
                std::slice::from_mut(msg),
            );
        }
    }
}

// ---- GUI chat（codex exec --json 实时事件流）---------------------------------
//
// 注意：这跟浏览模式（read_with_title_index 读落盘 rollout）是**两套完全不同的事件
// 形状**。GUI chat 走 `codex exec [resume <id>] --json` 的实时流：
//   {"type":"thread.started","thread_id":"<uuid>"}      → Init（回填 session id 供下轮 resume）
//   {"type":"turn.started"} / {"type":"item.started",…} → 忽略（进行中，不渲染半成品）
//   {"type":"item.completed","item":{"type":"agent_message","text":…}}                  → 助手文本
//   {"type":"item.completed","item":{"type":"command_execution","command",
//        "aggregated_output","exit_code",…}}                                            → shell 工具块
//   {"type":"turn.completed","usage":{…}}               → Result（一轮结束 + usage）
//
// 进程模型：codex exec 一轮一进程（跑完即退），靠 thread_id resume 续上下文 —— 对应
// §9 的 OneShotResume 路径。某轮异常退出而没发 Result 时，由 agent_chat.rs 兜底补失败。

/// 未识别 item 通用兜底时，tool_input 的 JSON 摘要最长字符数（避免巨型 blob，如整文件内容）。
const CHAT_ITEM_FALLBACK_CAP: usize = 1200;

/// codex exec `usage` → UsageSummary。codex 的 `input_tokens` 已含 `cached_input_tokens`，
/// `output_tokens` 通常已含 `reasoning_output_tokens`，所以 headline `total` 只取
/// input+output，不重复累加 cached / reasoning（后两者仅作明细保留）。
fn parse_exec_usage(u: &Value) -> UsageSummary {
    let input = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
    let cached = u
        .get("cached_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
    let reasoning = u
        .get("reasoning_output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
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

/// 把 `<thinking>...</thinking>` 标签拆成 `thinking` 块，其余部分保留为 `text` 块。
fn split_thinking_blocks(input: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("<thinking>") {
        let before = &rest[..start];
        if !before.trim().is_empty() {
            blocks.push(text_block("text", before.trim()));
        }
        rest = &rest[start + "<thinking>".len()..];
        if let Some(end) = rest.find("</thinking>") {
            let thinking = &rest[..end];
            if !thinking.trim().is_empty() {
                blocks.push(text_block("thinking", thinking.trim()));
            }
            rest = &rest[end + "</thinking>".len()..];
        } else {
            if !rest.trim().is_empty() {
                blocks.push(text_block("thinking", rest.trim()));
            }
            rest = "";
            break;
        }
    }
    if !rest.trim().is_empty() {
        blocks.push(text_block("text", rest.trim()));
    }
    blocks
}

/// 一个 `item.completed` 的 item → 一条 Msg。`agent_message` 是助手文本，
/// `command_execution` 渲染成 shell 工具块（命令 + 输出），`reasoning` 与浏览模式一致
/// 不单独渲染；其它类型（file_change / mcp_tool_call / web_search …）走通用兜底，
/// 渲染成截断的 tool_use 块 —— 宁可粗糙也不静默丢掉 codex 的动作。
fn chat_item_to_msg(item: &Value) -> Option<Msg> {
    match item.get("type").and_then(Value::as_str).unwrap_or("") {
        "agent_message" => {
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            if text.trim().is_empty() {
                return None;
            }
            let blocks = split_thinking_blocks(text);
            if blocks.is_empty() {
                return None;
            }
            Some(Msg {
                uuid: None,
                role: "assistant".to_string(),
                timestamp: None,
                model: None,
                sidechain: false,
                blocks,
                meta_kind: None,
            })
        }
        "reasoning" => None,
        "error" => {
            let msg = item.get("message").and_then(Value::as_str).unwrap_or("");
            if msg.is_empty() {
                return None;
            }
            // codex_hooks 弃用等纯信息性 CLI 警告：静默丢弃。
            if msg.contains("is deprecated") {
                return None;
            }
            Some(simple_msg(
                "assistant",
                None,
                text_block("system_event", msg),
            ))
        }
        "command_execution" => {
            let command = item.get("command").and_then(Value::as_str).unwrap_or("");
            let output = item
                .get("aggregated_output")
                .and_then(Value::as_str)
                .unwrap_or("");
            let exit_code = item.get("exit_code").and_then(Value::as_i64);
            let id = item.get("id").and_then(Value::as_str).map(str::to_string);
            let mut blocks = vec![Block {
                kind: "tool_use".to_string(),
                tool_name: Some("shell".to_string()),
                tool_input: Some(command.to_string()),
                tool_id: id.clone(),
                ..Default::default()
            }];
            if !output.trim().is_empty() || exit_code.is_some() {
                blocks.push(Block {
                    kind: "tool_result".to_string(),
                    text: Some(output.to_string()),
                    tool_id: id,
                    is_error: exit_code.map(|c| c != 0).unwrap_or(false),
                    ..Default::default()
                });
            }
            Some(Msg {
                uuid: None,
                role: "assistant".to_string(),
                timestamp: None,
                model: None,
                sidechain: false,
                blocks,
                meta_kind: None,
            })
        }
        other if !other.is_empty() => {
            let mut summary = serde_json::to_string(item).unwrap_or_default();
            if summary.chars().count() > CHAT_ITEM_FALLBACK_CAP {
                summary = summary
                    .chars()
                    .take(CHAT_ITEM_FALLBACK_CAP)
                    .collect::<String>()
                    + " …";
            }
            Some(simple_msg(
                "assistant",
                None,
                Block {
                    kind: "tool_use".to_string(),
                    tool_name: Some(other.to_string()),
                    tool_input: Some(summary),
                    ..Default::default()
                },
            ))
        }
        _ => None,
    }
}

/// 解析 codex exec `--json` 的一行事件 → 归一的 ChatEvent。
pub(crate) fn parse_chat_line(line: &str) -> ChatEvent {
    let line = line.trim();
    if line.is_empty() {
        return ChatEvent::Ignore;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return ChatEvent::Ignore;
    };
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "thread.started" => ChatEvent::Init {
            session_id: v
                .get("thread_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            // Codex 无 apiKeySource 概念（5h/周限额角标是 Claude 专属）。
            api_key_source: None,
        },
        "item.completed" => match v.get("item").and_then(chat_item_to_msg) {
            Some(msg) => ChatEvent::Message(msg),
            None => ChatEvent::Ignore,
        },
        "turn.completed" => ChatEvent::Result {
            ok: true,
            usage: v.get("usage").map(parse_exec_usage),
        },
        "turn.failed" | "error" => ChatEvent::Result {
            ok: false,
            usage: None,
        },
        _ => ChatEvent::Ignore,
    }
}

impl SessionSource for CodexSource {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn list_projects(
        &self,
        include_codex_internal: bool,
        include_codex_archived: bool,
    ) -> Result<Vec<ProjectInfo>, String> {
        let mut map: HashMap<String, (usize, u64)> = HashMap::new();
        let flags_index = load_thread_flags_index();
        for fp in all_files(include_codex_archived) {
            if let Some(m) = meta(&fp) {
                let flags = flags_for(&fp, &m, &flags_index);
                if !include_by_flags(flags, include_codex_internal, include_codex_archived) {
                    continue;
                }
                let mt = mtime_millis(&fp);
                let entry = map.entry(m.cwd).or_insert((0, 0));
                entry.0 += 1;
                if mt > entry.1 {
                    entry.1 = mt;
                }
            }
        }
        let mut out: Vec<ProjectInfo> = map
            .into_iter()
            .map(|(cwd, (count, last))| {
                let exists = Path::new(&cwd).is_dir();
                ProjectInfo {
                    dir_name: cwd.clone(),
                    display_path: cwd,
                    session_count: count,
                    last_modified: last,
                    exists,
                    bookmarked: false,
                    parent_dir_name: None,
                    worktree_name: None,
                }
            })
            .collect();
        out.sort_by_key(|p| std::cmp::Reverse(p.last_modified));
        Ok(out)
    }

    fn list_sessions(
        &self,
        project_key: &str,
        offset: usize,
        limit: usize,
        include_codex_internal: bool,
        include_codex_archived: bool,
    ) -> Result<SessionPage, String> {
        // 廉价阶段：只读每个文件首行 session_meta，筛出本项目的文件并取修改时间。
        let mut matched: Vec<(PathBuf, Meta, u64, CodexThreadFlags)> = Vec::new();
        let flags_index = load_thread_flags_index();
        for fp in all_files(include_codex_archived) {
            if let Some(m) = meta(&fp) {
                if m.cwd == project_key {
                    let flags = flags_for(&fp, &m, &flags_index);
                    if !include_by_flags(flags, include_codex_internal, include_codex_archived) {
                        continue;
                    }
                    let mt = mtime_millis(&fp);
                    matched.push((fp, m, mt, flags));
                }
            }
        }
        matched.sort_by_key(|m| std::cmp::Reverse(m.2));
        let total = matched.len();
        // Codex 把会话标题缓存在 ~/.codex/session_index.jsonl（append-only，同 id
        // 多条时最新一条胜出）。列表整页加载一次即可，避免每个会话都重读一次文件。
        let title_index = load_title_index();
        let mut sessions: Vec<SessionMeta> = matched
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(p, m, _, flags)| scan(p, m, &title_index, *flags))
            .collect();
        if limit != usize::MAX {
            let snapshot = query_codex_app_thread_list();
            apply_codex_app_list_snapshot(&mut sessions, &snapshot);
        }
        Ok(SessionPage { total, sessions })
    }

    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String> {
        let mut msgs = read(path)?;
        post_process_codex_session_msgs(&mut msgs);
        Ok(msgs)
    }

    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String> {
        let trimmed = validate_rename_name(name)?;
        let filename_id = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.trim_end_matches(".jsonl").to_string())
            .unwrap_or_default();

        // Codex 文件名形如 rollout-<ts>-<uuid>.jsonl，真正的 thread_id 在
        // 首行 session_meta.payload.id 里。
        let mut codex_id: Option<String> = None;
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok).take(8) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if v.get("type").and_then(|x| x.as_str()) == Some("session_meta") {
                        if let Some(idv) = v
                            .get("payload")
                            .and_then(|p| p.get("id"))
                            .and_then(|x| x.as_str())
                        {
                            codex_id = Some(idv.to_string());
                            break;
                        }
                    }
                }
            }
        }
        let codex_id = codex_id.unwrap_or(filename_id);

        // 1) 在 rollout JSONL 末尾追加 thread_name_updated 事件（跟 codex-tui 自己写的一致）。
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let secs = (now_ms / 1000) as i64;
        let ms = (now_ms % 1000) as u32;
        let ts = format_iso8601_utc(secs, ms);
        let line = serde_json::json!({
            "timestamp": ts,
            "type": "event_msg",
            "payload": {
                "type": "thread_name_updated",
                "thread_id": codex_id,
                "thread_name": trimmed,
            },
        })
        .to_string();
        append_jsonl_line(path, &line)?;

        // 2) 更新 ~/.codex/session_index.jsonl —— codex picker 读这个文件。
        // 实测：同 id 多条时 codex picker 取**首次出现**的那条（不是按 updated_at
        // 排序）。所以不能单纯 append，必须先把同 id 的旧条目过滤掉，再把新条目
        // 写到末尾——这样新 rename 一定能被读到，又跟 codex 自己的格式兼容。
        let updated_at = format_iso8601_utc(secs, ms).replace('Z', "000Z");
        let new_entry = serde_json::json!({
            "id": codex_id,
            "thread_name": trimmed,
            "updated_at": updated_at,
        })
        .to_string();

        let idx_path = home().join(".codex").join("session_index.jsonl");
        let mut retained: Vec<String> = Vec::new();
        if idx_path.exists() {
            if let Ok(file) = fs::File::open(&idx_path) {
                for line in BufReader::new(file).lines().map_while(Result::ok) {
                    let raw = line.trim_end_matches(['\r', '\n']);
                    if raw.is_empty() {
                        continue;
                    }
                    let same_id = serde_json::from_str::<Value>(raw)
                        .ok()
                        .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(str::to_owned))
                        .map(|id| id == codex_id)
                        .unwrap_or(false);
                    if !same_id {
                        retained.push(raw.to_string());
                    }
                }
            }
        }
        retained.push(new_entry);

        // 原子替换：先写到同目录下的临时文件，再 rename 覆盖
        let parent = idx_path
            .parent()
            .ok_or_else(|| "session_index parent directory does not exist".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create .codex directory: {e}"))?;
        let tmp_path = parent.join(format!(".session_index.{}.tmp", now_ms));
        {
            let mut tmp = fs::File::create(&tmp_path)
                .map_err(|e| format!("Failed to open session_index temp file: {e}"))?;
            for line in &retained {
                tmp.write_all(line.as_bytes())
                    .map_err(|e| format!("Failed to write session_index entry: {e}"))?;
                tmp.write_all(b"\n")
                    .map_err(|e| format!("Failed to write session_index newline: {e}"))?;
            }
            tmp.flush().map_err(|e| format!("flush failed: {e}"))?;
        }
        fs::rename(&tmp_path, &idx_path)
            .map_err(|e| format!("Failed to replace session_index: {e}"))?;

        // 3) 真正权威：~/.codex/state_<N>.sqlite 的 threads.title 列。
        // 如果只改 session_index.jsonl 不改 sqlite，picker 仍会显示旧 title。
        // 文件不存在则跳过（codex 旧版本 / 用户从未启动过 codex CLI）。
        if let Some(db_path) = find_state_db() {
            let now_secs = (now_ms / 1000) as i64;
            let conn = rusqlite::Connection::open(&db_path)
                .map_err(|e| format!("Failed to open codex sqlite: {e}"))?;
            conn.execute(
                "UPDATE threads SET title = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![trimmed, now_secs, &codex_id],
            )
            .map_err(|e| format!("Failed to update threads.title: {e}"))?;
        }
        Ok(())
    }

    fn trash_title(&self, path: &Path) -> String {
        first_user_text(path)
    }

    fn resume_command(&self, session_id: &str, _path: &str) -> AgentCommand {
        AgentCommand::new("codex").arg("resume").arg(session_id)
    }

    fn new_session_command(&self) -> AgentCommand {
        AgentCommand::new("codex")
    }

    fn image_src(&self, block: &Value) -> Option<String> {
        image_src(block)
    }

    fn usage_summary(&self, path: &str) -> Result<UsageSummary, String> {
        usage_summary(Path::new(path))
    }

    fn last_prompt(&self, path: &str) -> Result<Option<String>, String> {
        Ok(last_user_text(Path::new(path)))
    }

    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String> {
        Ok(read_turns(Path::new(path)))
    }

    fn chat_slash_commands(&self, cwd: &str) -> Vec<crate::types::SlashCommand> {
        use std::path::Path;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let proj_name = super::claude::project_basename(cwd);

        // 项目级 skills: .codex/skills/ 和 .agents/skills/
        let proj_codex = Path::new(cwd).join(".codex").join("skills");
        super::claude::scan_skills_dir(
            &proj_codex,
            "project",
            proj_name.as_deref(),
            None,
            &mut out,
            &mut seen,
        );
        let proj_agents = Path::new(cwd).join(".agents").join("skills");
        super::claude::scan_skills_dir(
            &proj_agents,
            "project",
            proj_name.as_deref(),
            None,
            &mut out,
            &mut seen,
        );

        // 用户级 skills: ~/.codex/skills/ 和 ~/.agents/skills/
        let user_codex = home().join(".codex").join("skills");
        super::claude::scan_skills_dir(&user_codex, "user", None, None, &mut out, &mut seen);
        let user_agents = home().join(".agents").join("skills");
        super::claude::scan_skills_dir(&user_agents, "user", None, None, &mut out, &mut seen);

        out
    }

    fn chat_process_model(&self) -> ChatProcessModel {
        // GUI 使用 Codex rich-client app-server；thread 设置在 app-server 里有粘性，
        // 切模型/effort/权限需 restart/resume 才可靠生效。
        ChatProcessModel::CodexAppServer
    }

    fn chat_turn_command(
        &self,
        session_id: Option<&str>,
        prompt: &str,
        permission_mode: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Option<AgentCommand> {
        // 首轮 `codex exec`，续轮 `codex exec resume <id>`（id 紧跟 resume，prompt 收尾）。
        let mut cmd = AgentCommand::new("codex").arg("exec");
        if let Some(id) = session_id {
            cmd = cmd.arg("resume").arg(id);
        }
        cmd = cmd.arg("--json").arg("--skip-git-repo-check");
        // 模型 / effort：每轮重新下发即免费即时生效。model 走 `-m`；effort 是
        // reasoning 档位，经 `-c model_reasoning_effort=`（low|medium|high|xhigh|max|ultra）。
        // None 走 config.toml 默认（不下发）。
        if let Some(m) = model {
            cmd = cmd.arg("-m").arg(m);
        }
        if let Some(e) = effort {
            cmd = cmd.arg("-c").arg(format!("model_reasoning_effort=\"{e}\""));
        }
        // 权限模式 → codex sandbox。统一用 `-c sandbox_mode=`：`exec` 和 `exec resume`
        // 两个子命令都接受它，而 `-s/--sandbox` 在 `exec resume` 上不存在。
        // Codex 前端独立四档：ask / approve / fullAccess / custom。
        // 另兼容 Claude 的旧值以防续聊/reconnect 期间残留旧值。
        match permission_mode {
            "ask" | "plan" => {
                cmd = cmd.arg("-c").arg("sandbox_mode=\"read-only\"");
            }
            "approve" | "default" | "acceptEdits" => {
                cmd = cmd.arg("-c").arg("sandbox_mode=\"workspace-write\"");
            }
            "fullAccess" | "bypassPermissions" => {
                cmd = cmd.arg("--dangerously-bypass-approvals-and-sandbox");
            }
            // custom / 其它 → 不传 sandbox 参数，使用 config.toml 设定
            _ => {}
        }
        Some(cmd.arg(prompt))
    }

    fn parse_chat_line(&self, line: &str) -> ChatEvent {
        parse_chat_line(line)
    }
}

// ---- read_turns（统计聚合用）---------------------------------------------
//
// Codex 的 JSONL 单遍。这里关键约定 —— **一次 OpenAI API 调用 = 一个 `token_count` 事件**
// （和 codeburn / 任何官方账单维度一致）。`response_item.function_call` /
// `custom_tool_call` / `web_search_call`、`event_msg.agent_message` 都是同一次 API
// 响应里返回的 content block，并不是各自独立计费；旧版把每个都算一次 call，
// 对 Today 这种长跑 codex 会话能把 calls 放大到 ~3x（典型例子：49 个真实
// token_count 事件被算成 143 次 call，cost 因为我们已经把累积 usage 灌到最后一个
// CallRecord，反而是对的；但 calls/turns/By Activity 全错位）。
//
// 流程：
//   - 起 turn：`event_msg.user_message` —— message 是干净文本
//   - 起 / 接 call：每读到一个 token_count，把自上次 token_count 以来累积的
//     pending 工具（function_call / web_search_call / patch_apply_end 等）合并到一条
//     **新的** CallRecord 里，per-event 算 token / cost 再 push 到当前 turn
//   - model：来源是 `turn_context` 事件的 `payload.model`（mid-session 可能切换，譬如
//     gpt-5.5 / gpt-5.3-codex，每次出现就更新 `model_hint`）。
//     旧 `session_meta` 里**没有** model 字段（`originator` 是 "codex-tui" 这种字符串），
//     所以历史代码全部走 fallback 拿到空串，导致 pricing 算出 $0 —— 这里必须读 turn_context。
//   - usage：codex 在 token_count.info 里同时给 `last_token_usage`（这次调用的 delta）
//     和 `total_token_usage`（自 session 开始累积）。优先取 last_*；老格式没 last_* 时
//     从 total_* 相对前一帧的差值还原。两个连续帧 total_tokens 相同 → 重复事件，跳过。
fn last_user_text(fp: &Path) -> Option<String> {
    let raw = fs::read(fp).ok()?;
    for line in raw.rsplit(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        // Codex user messages: event_msg wrapper or response_item.payload
        let em = v
            .get("event_msg")
            .or_else(|| v.get("payload"))
            .or_else(|| v.get("response_item").and_then(|r| r.get("message")));
        let Some(em) = em else { continue };
        if em.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let text = em
            .get("content")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter().find(|c| {
                    let t = c.get("type").and_then(Value::as_str).unwrap_or("");
                    t == "input_text" || t == "text"
                })
            })
            .and_then(|c| c.get("text").and_then(Value::as_str));
        if let Some(t) = text {
            let trimmed = t.trim_start();
            if trimmed.starts_with("<skill>")
                || trimmed.starts_with("<context>")
                || trimmed.starts_with("<environment_context>")
            {
                continue;
            }
            let clean = crate::util::truncate_subtitle(t);
            if !clean.is_empty() {
                return Some(clean);
            }
        }
    }
    None
}

fn read_turns(fp: &Path) -> Vec<Turn> {
    let file = match fs::File::open(fp) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut turns: Vec<Turn> = Vec::new();
    let mut cur: Option<Turn> = None;
    let mut project_path: String = String::new();
    let mut session_id: String = String::new();
    let mut model_hint: String = String::new();

    // pending 工具调用元数据：在两次 token_count 之间累积，token_count 来临时
    // 落到那一帧的 CallRecord 上。这样 By Tool / Shell / MCP 仍然完整。
    let mut pending_tools: Vec<String> = Vec::new();
    let mut pending_bash: Vec<String> = Vec::new();
    let mut pending_mcp: Vec<String> = Vec::new();
    let mut pending_spawn = false;

    // token_count 累积态 —— 用来判 dup（相同 total_tokens）和 last_token_usage 缺失
    // 时的差值还原。None 哨兵：第一帧永远通过（如果一开始 total_tokens=0 的话，
    // 用 0 初始化会把它误判为 dup）。
    let mut prev_cum_total: Option<u64> = None;
    let mut prev_usage: Option<CodexUsageFields> = None;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let payload = match v.get("payload") {
            Some(p) => p,
            None => continue,
        };
        let pt = payload.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let ts_ms = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso8601_ms)
            .unwrap_or(0);

        match (t, pt) {
            ("session_meta", _) => {
                if session_id.is_empty() {
                    if let Some(id) = payload.get("id").and_then(|x| x.as_str()) {
                        session_id = id.to_string();
                    }
                }
                if project_path.is_empty() {
                    if let Some(c) = payload.get("cwd").and_then(|x| x.as_str()) {
                        project_path = c.to_string();
                    }
                }
                if model_hint.is_empty() {
                    if let Some(m) = payload.get("model").and_then(|x| x.as_str()) {
                        model_hint = m.to_string();
                    }
                }
            }
            ("turn_context", _) => {
                if let Some(m) = payload.get("model").and_then(|x| x.as_str()) {
                    if !m.is_empty() {
                        model_hint = m.to_string();
                    }
                }
            }
            ("event_msg", "user_message") => {
                if let Some(prev) = cur.take() {
                    turns.push(prev);
                }
                let text = payload
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                cur = Some(Turn {
                    user_message: text.to_string(),
                    project_path: project_path.clone(),
                    session_id: session_id.clone(),
                    calls: Vec::new(),
                    timestamp_ms: ts_ms,
                });
                // user_message 边界顺手清掉残留 pending（前一 turn 没收到 token_count
                // 就被新 user 打断的场景，丢掉这些 pending 不算钱也不算 call）。
                pending_tools.clear();
                pending_bash.clear();
                pending_mcp.clear();
                pending_spawn = false;
            }
            ("event_msg", "item_completed")
                if payload
                    .get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("UserMessage") =>
            {
                if let Some(prev) = cur.take() {
                    turns.push(prev);
                }
                let text = content_text(payload.get("item").and_then(|item| item.get("content")))
                    .unwrap_or_default();
                cur = Some(Turn {
                    user_message: text,
                    project_path: project_path.clone(),
                    session_id: session_id.clone(),
                    calls: Vec::new(),
                    timestamp_ms: ts_ms,
                });
                pending_tools.clear();
                pending_bash.clear();
                pending_mcp.clear();
                pending_spawn = false;
            }
            ("event_msg", "agent_message") => {
                // assistant 文本回复 —— 不单独计 call，等下一个 token_count 把它和
                // 工具调用一并并入一条 CallRecord 里。
            }
            ("response_item", "function_call") | ("response_item", "custom_tool_call") => {
                let name = payload
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let raw_args = payload
                    .get("arguments")
                    .or_else(|| payload.get("input"))
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                if name == "shell" || name == "Bash" || name == "BashTool" {
                    if let Some(cmd) = shell_util::extract_first_command(&raw_args) {
                        pending_bash.push(cmd);
                    }
                }
                if let Some(server) = shell_util::extract_mcp_server(&name) {
                    pending_mcp.push(server);
                }
                if matches!(name.as_str(), "Task" | "Agent" | "task_spawn") {
                    pending_spawn = true;
                }
                pending_tools.push(name);
            }
            ("response_item", "web_search_call") => {
                pending_tools.push("WebSearch".to_string());
            }
            ("event_msg", "token_count") => {
                let Some(info) = payload.get("info") else {
                    continue;
                };
                if info.is_null() {
                    continue;
                }
                let Some(tt) = info.get("total_token_usage") else {
                    continue;
                };
                let cum_total = tt.get("total_tokens").and_then(Value::as_u64).unwrap_or(0);

                // Codex 会在 rollout 中插入“新 task / resume”边界，此时累计值会从较小
                // 的新基线重新开始。不能把这次重置当作负增量，也不能把新 task 的完整
                // 快照丢掉；按新的基线继续累计即可。
                let is_reset = prev_cum_total.is_some_and(|prev| cum_total < prev);
                if !is_reset && prev_cum_total == Some(cum_total) {
                    continue;
                }
                if is_reset {
                    prev_usage = None;
                }
                prev_cum_total = Some(cum_total);

                let current_usage = codex_usage_fields(tt);
                let usage = codex_usage_delta(current_usage, prev_usage.unwrap_or((0, 0, 0, 0, 0)));
                prev_usage = Some(current_usage);

                if usage.total == 0 {
                    continue;
                }

                let mut call = CallRecord {
                    call_count: 1,
                    model: model_hint.clone(),
                    message_id: None,
                    usage,
                    cost_usd: 0.0,
                    pricing_missing: false,
                    pricing_estimated: false,
                    cost_source: crate::stats::types::CostSource::Unknown,
                    tools: std::mem::take(&mut pending_tools),
                    bash_commands: std::mem::take(&mut pending_bash),
                    mcp_servers: std::mem::take(&mut pending_mcp),
                    has_plan_mode: false,
                    has_agent_spawn: std::mem::replace(&mut pending_spawn, false),
                };
                call.cost_usd = pricing::cost_usd(&call.model, &call.usage);
                push_call(&mut cur, &project_path, &session_id, ts_ms, call);
            }
            _ => {}
        }
    }
    if let Some(t) = cur {
        turns.push(t);
    }
    turns
}

/// 起 / 追加一个 call —— 没有进行中的 user-turn 时起一个空 user_message 的占位。
fn push_call(
    cur: &mut Option<Turn>,
    project_path: &str,
    session_id: &str,
    ts_ms: i64,
    call: CallRecord,
) {
    if let Some(turn) = cur.as_mut() {
        turn.calls.push(call);
    } else {
        *cur = Some(Turn {
            user_message: String::new(),
            project_path: project_path.to_string(),
            session_id: session_id.to_string(),
            calls: vec![call],
            timestamp_ms: ts_ms,
        });
    }
}

/// Codex 把 token 用量写在 event_msg.token_count 事件里。通常是累积值，
/// 但跨 task / resume 时会回到新的基线；这里按快照差分并跨基线累加。
///
/// 形状：
///   {"type":"event_msg","payload":{"type":"token_count","info":{
///       "total_token_usage":{"input_tokens":N,"cached_input_tokens":N,
///         "output_tokens":N,"reasoning_output_tokens":N,"total_tokens":N},
///       ...}}}
///
/// 早期写入时 `info` 可能为 null（订阅尚未拿到 usage），跳过；后续的覆盖前面的。
fn usage_summary(fp: &Path) -> Result<UsageSummary, String> {
    let file = match fs::File::open(fp) {
        Ok(f) => f,
        Err(_) => return Ok(UsageSummary::default()),
    };
    let mut total = UsageSummary::default();
    let mut previous: Option<CodexUsageFields> = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let payload = match v.get("payload") {
            Some(p) => p,
            None => continue,
        };
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let Some(info) = payload.get("info") else {
            continue;
        };
        if info.is_null() {
            continue;
        }
        let Some(t) = info.get("total_token_usage") else {
            continue;
        };
        let current = codex_usage_fields(t);
        let is_reset = previous.is_some_and(|prev| current.4 < prev.4);
        if !is_reset && previous == Some(current) {
            continue;
        }
        let delta = codex_usage_delta(
            current,
            if is_reset {
                (0, 0, 0, 0, 0)
            } else {
                previous.unwrap_or((0, 0, 0, 0, 0))
            },
        );
        total.add_assign(&delta);
        previous = Some(current);
    }
    Ok(total.finalize())
}

type CodexUsageFields = (u64, u64, u64, u64, u64);

fn codex_usage_fields(t: &Value) -> CodexUsageFields {
    (
        t.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        t.get("cached_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        t.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        t.get("reasoning_output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        t.get("total_tokens").and_then(Value::as_u64).unwrap_or(0),
    )
}

/// Convert two cumulative Codex snapshots into one per-call UsageSummary.
/// `input_tokens` includes cached input, so the uncached portion is differenced
/// after subtracting the cached delta.
fn codex_usage_delta(current: CodexUsageFields, previous: CodexUsageFields) -> UsageSummary {
    let (input, cached, output, reasoning, _) = current;
    let (prev_input, prev_cached, prev_output, prev_reasoning, _) = previous;
    UsageSummary {
        input_tokens: input
            .saturating_sub(cached)
            .saturating_sub(prev_input.saturating_sub(prev_cached)),
        output_tokens: output.saturating_sub(prev_output),
        cache_creation_input_tokens: 0,
        cache_creation_1h_input_tokens: 0,
        cache_read_input_tokens: cached.saturating_sub(prev_cached),
        reasoning_output_tokens: reasoning.saturating_sub(prev_reasoning),
        total: 0,
    }
    .finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn codex_files_mentioned_block_extracts_files_and_request() {
        let text = "\n# Files mentioned by the user:\n\n## devtools_options.yaml: /Users/wuchao/develop/flutter/sales-app/devtools_options.yaml\n\n## My request for Codex:\nhi\n";
        let (files, body) = extract_codex_files(text);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, "file");
        assert_eq!(
            files[0].file_path.as_deref(),
            Some("/Users/wuchao/develop/flutter/sales-app/devtools_options.yaml")
        );
        assert_eq!(body, "hi");
    }

    #[test]
    fn codex_files_mentioned_multiple_files() {
        let text = "# Files mentioned by the user:\n\n## a.yaml: /p/a.yaml\n## b.json: /p/b.json\n\n## My request for Codex:\ncompare them\n";
        let (files, body) = extract_codex_files(text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_path.as_deref(), Some("/p/a.yaml"));
        assert_eq!(files[1].file_path.as_deref(), Some("/p/b.json"));
        assert_eq!(body, "compare them");
    }

    #[test]
    fn codex_plain_message_untouched() {
        let (files, body) = extract_codex_files("just a normal question\n");
        assert!(files.is_empty());
        assert_eq!(body, "just a normal question\n");
    }

    #[test]
    fn read_codex_keeps_plain_document_paths_in_user_message_as_text() {
        let mac_path = "/Users/wuchao/develop/protocol/ningxiaoshe/docs/更新记录/_index.md";
        let windows_path = r#"C:\Users\Jane Doe\Documents\_index.md"#;
        let text = format!("需要查看 PRD：{mac_path}，Windows 版本：{windows_path}");
        let line = json!({
            "type": "event_msg",
            "payload": { "type": "user_message", "message": text }
        })
        .to_string();
        let path = write_temp("codex-plain-document-paths.jsonl", &[&line]);

        let msgs = CodexSource
            .read_session(path.to_string_lossy().as_ref())
            .expect("Codex rollout should parse");
        let user = msgs
            .iter()
            .find(|msg| msg.role == "user")
            .expect("user message");

        assert!(user.blocks.iter().all(|block| block.kind != "file"));
        assert!(user.blocks.iter().any(|block| {
            block.kind == "text"
                && block
                    .text
                    .as_deref()
                    .is_some_and(|body| body.contains(mac_path) && body.contains(windows_path))
        }));
    }

    #[test]
    fn legacy_codex_pasted_image_path_gets_the_visible_image_tag() {
        let pasted = std::env::temp_dir()
            .join("cc-sessions-viewer-images")
            .join("chat-img-123.png");
        let mut blocks = vec![
            Block {
                kind: "image".to_string(),
                image_src: Some("/Users/wuchao/Downloads/ordinary.png".to_string()),
                ..Default::default()
            },
            Block {
                kind: "image".to_string(),
                image_src: Some(pasted.to_string_lossy().to_string()),
                ..Default::default()
            },
        ];
        bind_codex_pasted_file_placeholders("hi [Image #1]", &mut blocks);
        assert_eq!(blocks[0].inline_placeholder, None);
        assert_eq!(blocks[1].inline_placeholder.as_deref(), Some("[Image #1]"));
    }

    #[test]
    fn parses_view_image_windows_path_from_javascript_tool_input() {
        let input = r#"const r = await tools.view_image({path:"C:\\Users\\Jane\\Temp\\screen.png",detail:"high"}); image(r.image_url)"#;
        assert_eq!(
            tool_input_path(input).as_deref(),
            Some(r#"C:\Users\Jane\Temp\screen.png"#)
        );
    }

    #[test]
    fn hydrates_stale_codex_image_file_and_preserves_hash_image_token() {
        let path = r#"C:\Users\Jane\AppData\Local\Temp\screen.png"#;
        let user_text = format!(
            "# Files mentioned by the user:\n\n## {path}: {path}\n\n## My request for Codex:\n[#image 1] hi"
        );
        let escaped_path = path.replace('\\', "\\\\");
        let lines = [
            json!({
                "type": "session_meta",
                "payload": { "id": "stale-image", "cwd": "C:\\repo" }
            })
            .to_string(),
            json!({
                "type": "event_msg",
                "payload": { "type": "user_message", "message": user_text }
            })
            .to_string(),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "call_id": "view-1",
                    "name": "exec",
                    "input": format!(
                        "const r = await tools.view_image({{path:\"{escaped_path}\",detail:\"high\"}}); image(r.image_url)"
                    )
                }
            })
            .to_string(),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call_output",
                    "call_id": "view-1",
                    "output": [
                        { "type": "input_image", "image_url": "data:image/png;base64,abc" }
                    ]
                }
            })
            .to_string(),
        ];
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let p = write_temp("codex-stale-image-hydration.jsonl", &refs);
        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &HashMap::new()).unwrap();
        let user = msgs.iter().find(|msg| msg.role == "user").unwrap();
        assert_eq!(user.blocks[0].kind, "image");
        assert_eq!(
            user.blocks[0].image_src.as_deref(),
            Some("data:image/png;base64,abc")
        );
        assert_eq!(
            user.blocks[0].inline_placeholder.as_deref(),
            Some("[#image 1]")
        );
        assert_eq!(user.blocks[1].text.as_deref(), Some("[#image 1] hi"));
    }

    #[test]
    fn read_new_codex_rollout_message_shape() {
        let lines = [
            r#"{"timestamp":"2026-08-12T02:52:01.262Z","type":"session_meta","payload":{"id":"new","cwd":"/tmp"}}"#,
            r#"{"timestamp":"2026-08-12T02:52:02.576Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,abc"},{"type":"input_text","text":"<image name=[Image #1] path=\"/tmp/a.png\">\n</image>\n[Image #1] 给宠物增加右键菜单，看截图"}]}}"#,
            r#"{"timestamp":"2026-08-12T02:52:02.577Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"u1","content":[{"type":"local_image","path":"/tmp/a.png"},{"type":"text","text":"[Image #1] 给宠物增加右键菜单，看截图"}]}}}"#,
            r#"{"timestamp":"2026-08-12T02:52:03.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","id":"a1","phase":"final_answer","content":[{"type":"text","text":"已处理"}]}}}"#,
        ];
        let p = write_temp("codex-new-message-shape.jsonl", &lines);
        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &HashMap::new()).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].blocks[0].kind, "image");
        assert_eq!(
            msgs[0].blocks[1].text.as_deref(),
            Some("[Image #1] 给宠物增加右键菜单，看截图")
        );
        assert_eq!(msgs[1].blocks[0].text.as_deref(), Some("已处理"));

        let meta = meta(&p).expect("meta");
        let session = scan(&p, &meta, &HashMap::new(), CodexThreadFlags::default());
        assert_eq!(session.title, "[Image #1] 给宠物增加右键菜单，看截图");
        assert_eq!(session.message_count, 2);
    }

    #[cfg(windows)]
    #[test]
    fn read_codex_keeps_example_windows_paths_in_protocol_image_message_as_text() {
        let first = r#"C:\Users\Jane Doe\AppData\Local\Temp\pi-clipboard-first.png"#;
        let second = r#"C:\Users\Jane Doe\AppData\Local\Temp\pi-clipboard-second.png"#;
        let image_header =
            r#"<image name=[Image #1] path="C:\Users\Jane Doe\AppData\Local\Temp\codex-clipboard.png">"#;
        let text = format!("hi, {first}, ooo, {second}, this is an image test [Image #1]");
        let lines = [
            json!({
                "type": "session_meta",
                "payload": { "id": "codex-windows-path-prose", "cwd": "C:\\repo" }
            })
            .to_string(),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": image_header },
                        { "type": "input_image", "image_url": "data:image/png;base64,abc" },
                        { "type": "input_text", "text": "</image>" },
                        { "type": "input_text", "text": text }
                    ]
                }
            })
            .to_string(),
            json!({
                "type": "event_msg",
                "payload": {
                    "type": "item_completed",
                    "item": {
                        "type": "UserMessage",
                        "id": "u1",
                        "content": [{ "type": "text", "text": text }]
                    }
                }
            })
            .to_string(),
        ];
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let path = write_temp("codex-windows-path-prose.jsonl", &refs);
        let msgs = CodexSource
            .read_session(path.to_string_lossy().as_ref())
            .expect("Codex rollout should parse");
        let user = msgs
            .iter()
            .find(|msg| msg.role == "user")
            .expect("user message");
        let image = user
            .blocks
            .iter()
            .find(|block| block.kind == "image")
            .expect("protocol image block");
        assert_eq!(
            image.image_src.as_deref(),
            Some("data:image/png;base64,abc")
        );
        assert_eq!(image.inline_placeholder.as_deref(), Some("[Image #1]"));

        assert_eq!(
            user.blocks
                .iter()
                .filter(|block| block.kind == "image")
                .count(),
            1
        );
        assert_eq!(
            user.blocks
                .iter()
                .filter(|block| block.kind == "file")
                .count(),
            0
        );
        assert!(user.blocks.iter().any(|block| {
            block.kind == "text"
                && block.text.as_deref().is_some_and(|body| {
                    body.contains(first) && body.contains(second) && body.contains("[Image #1]")
                })
        }));
    }

    #[test]
    fn read_codex_preserves_mixed_file_and_image_attachment_positions() {
        let lines = [
            r#"{"timestamp":"2026-08-12T02:52:01.000Z","type":"session_meta","payload":{"id":"mixed","cwd":"/tmp"}}"#,
            r#"{"timestamp":"2026-08-12T02:52:02.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<image name=[Image #2] path=\"/tmp/pasted.png\">\n"},{"type":"input_image","image_url":"data:image/png;base64,abc"},{"type":"input_text","text":"</image>"},{"type":"input_text","text":"[Image #2] review"}]}}"#,
            r##"{"timestamp":"2026-08-12T02:52:02.001Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"u1","content":[{"type":"local_image","path":"/tmp/pasted.png"},{"type":"text","text":"# Files mentioned by the user:\n\n## report.pdf: /tmp/report.pdf\n\n## My request for Codex:\n[Image #2] review"}]}}}"##,
        ];
        let p = write_temp("codex-mixed-attachment-order.jsonl", &lines);
        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &HashMap::new()).unwrap();
        let blocks = &msgs[0].blocks;
        assert_eq!(blocks[0].kind, "file");
        assert_eq!(blocks[0].file_path.as_deref(), Some("/tmp/report.pdf"));
        assert_eq!(blocks[1].kind, "image");
        assert_eq!(blocks[1].inline_placeholder.as_deref(), Some("[Image #2]"));
        assert_eq!(blocks[2].text.as_deref(), Some("[Image #2] review"));
    }

    #[test]
    fn read_new_codex_agent_message_text_block_shape() {
        let p = write_temp(
            "codex-new-agent-text-shape.jsonl",
            &[
                r#"{"timestamp":"2026-08-12T02:52:03.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","id":"a1","phase":"final_answer","content":[{"type":"Text","text":"已处理"}]}}}"#,
            ],
        );
        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &HashMap::new()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].blocks[0].text.as_deref(), Some("已处理"));
    }

    #[test]
    fn read_turns_handles_codex_usage_reset_between_tasks() {
        crate::stats::pricing::seed_test_prices();
        let p = write_temp(
            "codex-usage-reset.jsonl",
            &[
                r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp"}}"#,
                r#"{"type":"turn_context","payload":{"model":"gpt-5"}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"one"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":110}}}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"two"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":0,"output_tokens":5,"reasoning_output_tokens":0,"total_tokens":55},"last_token_usage":{"input_tokens":50,"cached_input_tokens":0,"output_tokens":5,"reasoning_output_tokens":0,"total_tokens":55}}}}"#,
            ],
        );
        let turns = read_turns(&p);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].calls[0].usage.total, 110);
        assert_eq!(turns[1].calls[0].usage.total, 55);
    }

    #[test]
    fn read_session_hides_deferred_steer_control_text() {
        let deferred = "[Deferred follow-up from the user]\n\
Continue and fully complete the original task already in progress first.\n\
Do not interrupt it, change its direction, or begin the follow-up below yet.\n\
Only after the original task is complete, process this follow-up in the order received:\n\
<follow-up>\n当前时间\n</follow-up>";
        let line = serde_json::json!({
            "timestamp": "2026-08-03T03:00:00.000Z",
            "type": "event_msg",
            "payload": { "type": "user_message", "message": deferred },
        })
        .to_string();
        let path = write_temp("codex-deferred-steer.jsonl", &[&line]);
        let msgs = read_with_title_index(path.to_string_lossy().as_ref(), &HashMap::new())
            .expect("read deferred steer session");

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].blocks[0].text.as_deref(), Some("当前时间"));
    }

    #[test]
    fn strip_edge_at_paths_keeps_inline_prompt_reference() {
        let text = "分析@scripts/release/appstore-release.sh的执行过程和调用命令";
        assert_eq!(strip_edge_at_paths(text), text);
    }

    #[test]
    fn strip_edge_at_paths_removes_edge_attachment_references() {
        assert_eq!(
            strip_edge_at_paths(
                "@\"/tmp/image.png\" 分析@scripts/a.sh的执行过程 @\"/tmp/end.png\""
            ),
            "分析@scripts/a.sh的执行过程"
        );
    }

    fn write_temp(name: &str, lines: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("csv-codex-usage-tests");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        p
    }

    // ---- GUI chat（codex exec --json 实时事件流）协议解析 ----

    #[test]
    fn chat_thread_started_becomes_init_with_thread_id() {
        let line =
            r#"{"type":"thread.started","thread_id":"019f01ae-6cd3-7493-b29c-243ab87ecf28"}"#;
        match parse_chat_line(line) {
            ChatEvent::Init { session_id, .. } => {
                assert_eq!(
                    session_id.as_deref(),
                    Some("019f01ae-6cd3-7493-b29c-243ab87ecf28")
                );
            }
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn chat_agent_message_item_becomes_assistant_text() {
        let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"banana42"}}"#;
        match parse_chat_line(line) {
            ChatEvent::Message(m) => {
                assert_eq!(m.role, "assistant");
                assert_eq!(m.blocks.len(), 1);
                assert_eq!(m.blocks[0].kind, "text");
                assert_eq!(m.blocks[0].text.as_deref(), Some("banana42"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn chat_command_execution_item_becomes_shell_tool_use_plus_result() {
        let line = r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"echo hi","aggregated_output":"hi\n","exit_code":0,"status":"completed"}}"#;
        match parse_chat_line(line) {
            ChatEvent::Message(m) => {
                assert_eq!(m.role, "assistant");
                assert_eq!(m.blocks.len(), 2);
                assert_eq!(m.blocks[0].kind, "tool_use");
                assert_eq!(m.blocks[0].tool_name.as_deref(), Some("shell"));
                assert_eq!(m.blocks[0].tool_input.as_deref(), Some("echo hi"));
                assert_eq!(m.blocks[0].tool_id.as_deref(), Some("item_0"));
                assert_eq!(m.blocks[1].kind, "tool_result");
                assert_eq!(m.blocks[1].text.as_deref(), Some("hi\n"));
                assert!(!m.blocks[1].is_error);
                assert_eq!(m.blocks[1].tool_id.as_deref(), Some("item_0"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn chat_command_execution_nonzero_exit_marks_error() {
        let line = r#"{"type":"item.completed","item":{"id":"item_2","type":"command_execution","command":"false","aggregated_output":"","exit_code":1,"status":"completed"}}"#;
        match parse_chat_line(line) {
            ChatEvent::Message(m) => {
                // 输出为空但 exit_code 存在 → 仍补一条 tool_result，并标记 is_error。
                assert_eq!(m.blocks.len(), 2);
                assert!(m.blocks[1].is_error);
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn chat_unknown_item_falls_back_to_tool_use_not_dropped() {
        let line = r#"{"type":"item.completed","item":{"id":"i","type":"file_change","status":"completed"}}"#;
        match parse_chat_line(line) {
            ChatEvent::Message(m) => {
                assert_eq!(m.blocks[0].kind, "tool_use");
                assert_eq!(m.blocks[0].tool_name.as_deref(), Some("file_change"));
            }
            _ => panic!("expected fallback Message"),
        }
    }

    #[test]
    fn chat_reasoning_item_is_ignored() {
        let line = r#"{"type":"item.completed","item":{"type":"reasoning","text":"thinking..."}}"#;
        assert!(matches!(parse_chat_line(line), ChatEvent::Ignore));
    }

    #[test]
    fn chat_turn_completed_is_ok_with_mapped_usage() {
        let line = r#"{"type":"turn.completed","usage":{"input_tokens":25790,"cached_input_tokens":14080,"output_tokens":335,"reasoning_output_tokens":197}}"#;
        match parse_chat_line(line) {
            ChatEvent::Result { ok, usage } => {
                assert!(ok);
                let u = usage.expect("usage present");
                assert_eq!(u.input_tokens, 25790);
                assert_eq!(u.output_tokens, 335);
                assert_eq!(u.cache_read_input_tokens, 14080);
                assert_eq!(u.reasoning_output_tokens, 197);
                // headline total 不重复计 cached（含于 input）/ reasoning（含于 output）。
                assert_eq!(u.total, 25790 + 335);
            }
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn chat_turn_started_and_garbage_are_ignored() {
        assert!(matches!(
            parse_chat_line(r#"{"type":"turn.started"}"#),
            ChatEvent::Ignore
        ));
        assert!(matches!(
            parse_chat_line(r#"{"type":"item.started","item":{"type":"command_execution"}}"#),
            ChatEvent::Ignore
        ));
        assert!(matches!(parse_chat_line("not json"), ChatEvent::Ignore));
        assert!(matches!(parse_chat_line(""), ChatEvent::Ignore));
    }

    #[test]
    fn chat_turn_command_new_turn_uses_exec_with_workspace_write() {
        let cmd = <CodexSource as crate::agents::SessionSource>::chat_turn_command(
            &CodexSource,
            None,
            "hello world",
            "approve",
            None,
            None,
        )
        .expect("codex supports chat");
        let shell = cmd.to_posix_shell();
        assert!(shell.contains("'exec'"), "{shell}");
        assert!(
            !shell.contains("'resume'"),
            "new turn must not resume: {shell}"
        );
        assert!(shell.contains("'--json'"));
        assert!(shell.contains("'--skip-git-repo-check'"));
        assert!(
            shell.contains(r#"'sandbox_mode="workspace-write"'"#),
            "{shell}"
        );
        assert!(
            shell.ends_with("'hello world'"),
            "prompt is the trailing positional: {shell}"
        );
    }

    #[test]
    fn chat_turn_command_resume_puts_id_before_prompt() {
        let cmd = <CodexSource as crate::agents::SessionSource>::chat_turn_command(
            &CodexSource,
            Some("019f-abc"),
            "again",
            "approve",
            None,
            None,
        )
        .expect("codex supports chat");
        let args = cmd.args();
        // exec resume <id> …options… <prompt>
        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "resume");
        assert_eq!(args[2], "019f-abc");
        assert_eq!(args.last().unwrap(), "again");
    }

    #[test]
    fn chat_turn_command_permission_mode_maps_to_sandbox() {
        // ask → sandbox_mode="read-only"
        let ask = <CodexSource as crate::agents::SessionSource>::chat_turn_command(
            &CodexSource,
            None,
            "p",
            "ask",
            None,
            None,
        )
        .unwrap()
        .to_posix_shell();
        assert!(ask.contains(r#"'sandbox_mode="read-only"'"#), "{ask}");

        // fullAccess → --dangerously-bypass-approvals-and-sandbox
        let full = <CodexSource as crate::agents::SessionSource>::chat_turn_command(
            &CodexSource,
            None,
            "p",
            "fullAccess",
            None,
            None,
        )
        .unwrap()
        .to_posix_shell();
        assert!(
            full.contains("'--dangerously-bypass-approvals-and-sandbox'"),
            "{full}"
        );

        // custom → no sandbox_mode
        let custom = <CodexSource as crate::agents::SessionSource>::chat_turn_command(
            &CodexSource,
            None,
            "p",
            "custom",
            None,
            None,
        )
        .unwrap()
        .to_posix_shell();
        assert!(
            !custom.contains("sandbox_mode"),
            "custom must not pass sandbox: {custom}"
        );
        assert!(
            !custom.contains("--dangerously-bypass"),
            "custom must not bypass: {custom}"
        );
    }

    #[test]
    fn chat_turn_command_model_and_effort_emit_flags() {
        let cmd = <CodexSource as crate::agents::SessionSource>::chat_turn_command(
            &CodexSource,
            None,
            "hi",
            "acceptEdits",
            Some("gpt-5.1-codex-max"),
            Some("high"),
        )
        .unwrap()
        .to_posix_shell();
        assert!(cmd.contains("'-m' 'gpt-5.1-codex-max'"), "{cmd}");
        assert!(cmd.contains(r#"'model_reasoning_effort="high"'"#), "{cmd}");
    }

    #[test]
    fn chat_turn_command_no_model_effort_emits_no_flags() {
        let cmd = <CodexSource as crate::agents::SessionSource>::chat_turn_command(
            &CodexSource,
            None,
            "hi",
            "acceptEdits",
            None,
            None,
        )
        .unwrap()
        .to_posix_shell();
        assert!(!cmd.contains("'-m'"), "no model flag when None: {cmd}");
        assert!(
            !cmd.contains("model_reasoning_effort"),
            "no effort flag when None: {cmd}"
        );
    }

    #[test]
    fn usage_takes_the_last_non_null_token_count_event() {
        // 早期 info:null 的事件被跳过；后面的累积值（total_token_usage）覆盖。
        // input_tokens 字段 codex 报的是"含 cached"的总输入，本函数会减出来。
        let p = write_temp(
            "codex-last.jsonl",
            &[
                r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":30,"output_tokens":40,"reasoning_output_tokens":20,"total_tokens":190}}}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200,"cached_input_tokens":60,"output_tokens":80,"reasoning_output_tokens":35,"total_tokens":375}}}}"#,
            ],
        );
        let u = usage_summary(&p).unwrap();
        // 最后一条 total: input=200 含 60 cached → new input = 140
        assert_eq!(u.input_tokens, 140);
        assert_eq!(u.cache_read_input_tokens, 60);
        assert_eq!(u.output_tokens, 80);
        assert_eq!(u.reasoning_output_tokens, 35);
        // total = (200-60) + 80 + 60 (cache_read) + 0 (cache_creation) + 35 (reasoning) = 315
        assert_eq!(u.total, 315);
    }

    #[test]
    fn usage_handles_cached_greater_than_input_defensively() {
        // 防御性：API 抖动时 cached > input —— new input 应该按 0 处理而不是 panic。
        let p = write_temp(
            "codex-defensive.jsonl",
            &[
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":100,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":160}}}}"#,
            ],
        );
        let u = usage_summary(&p).unwrap();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 100);
    }

    #[test]
    fn usage_ignores_unrelated_events() {
        let p = write_temp(
            "codex-noise.jsonl",
            &[
                r#"{"type":"response_item","payload":{"type":"message"}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message"}}"#,
            ],
        );
        assert_eq!(usage_summary(&p).unwrap(), UsageSummary::default());
    }

    #[test]
    fn usage_returns_default_when_file_missing() {
        let p = std::path::PathBuf::from("/tmp/csv-codex-usage-tests/nope.jsonl");
        assert_eq!(usage_summary(&p).unwrap(), UsageSummary::default());
    }

    #[test]
    fn thread_flags_do_not_treat_missing_user_event_alone_as_internal() {
        let flags =
            thread_flags_from_fields(false, 0, r#"{"local":true}"#, None, Some("gpt-5-codex"));
        assert!(!flags.internal);
        assert!(!flags.archived);
    }

    #[test]
    fn thread_flags_detect_guardian_subagent_and_archive_independently() {
        let flags = thread_flags_from_fields(
            true,
            0,
            r#"{"subagent":{"other":"guardian"}}"#,
            Some("subagent"),
            Some("codex-auto-review"),
        );
        assert!(flags.internal);
        assert!(flags.archived);
        assert!(include_by_flags(flags, false, true));
    }

    #[test]
    fn read_turns_picks_up_model_from_turn_context_so_cost_is_nonzero() {
        // 回归：早期实现只看 session_meta.originator.model / .model；真实 codex JSONL 的
        // session_meta.originator 是字符串（"codex-tui"），model 字段不存在 → 全 session $0。
        // 现在 turn_context.payload.model 是真正的 model 源。
        crate::stats::pricing::seed_test_prices();
        let p = write_temp(
            "codex-turn-context-model.jsonl",
            &[
                r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp","originator":"codex-tui"}}"#,
                r#"{"type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5"}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#,
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"hey"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":500,"reasoning_output_tokens":0,"total_tokens":1500}}}}"#,
            ],
        );
        let turns = read_turns(&p);
        let last_call = turns
            .last()
            .and_then(|t| t.calls.last())
            .expect("expected at least one call");
        assert_eq!(last_call.model, "gpt-5");
        assert!(
            last_call.cost_usd > 0.0,
            "expected non-zero cost, got {}",
            last_call.cost_usd
        );
    }

    #[test]
    fn read_turns_counts_one_call_per_token_count_not_per_tool_invocation() {
        // 回归：一个 turn 里 OpenAI 通常一次 API 响应同时返回多个 function_call /
        // agent_message —— 那些是同一次 API 调用的 content blocks，**不是各自计费的**
        // 独立调用。曾把每个 function_call 都算成一次 call，导致 Today 维度 calls 数
        // 被放大到 ~3x（同一份 codex JSONL：49 个真实 token_count → 我们报 143 calls）。
        // 期望：一个 turn 里 3 个 function_call + 1 个 agent_message + 1 个 token_count
        // → 一条 CallRecord，且 tools/bash 全部归并到这条上。
        crate::stats::pricing::seed_test_prices();
        let p = write_temp(
            "codex-one-call-per-token-count.jsonl",
            &[
                r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp","originator":"codex-tui"}}"#,
                r#"{"type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5"}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":\"ls /tmp\"}"}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":\"pwd\"}"}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call","name":"read_file","arguments":"{\"path\":\"a.txt\"}"}}"#,
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"done"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":500,"reasoning_output_tokens":0,"total_tokens":1700},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":500,"reasoning_output_tokens":0,"total_tokens":1700}}}}"#,
            ],
        );
        let turns = read_turns(&p);
        assert_eq!(turns.len(), 1, "expected exactly 1 turn");
        let calls = &turns[0].calls;
        assert_eq!(
            calls.len(),
            1,
            "expected exactly 1 CallRecord (one per token_count), got {} — they're being counted per function_call again",
            calls.len(),
        );
        let c = &calls[0];
        assert_eq!(
            c.tools.len(),
            3,
            "all 3 tool names should be folded into the call"
        );
        assert!(c.tools.contains(&"shell".to_string()));
        assert!(c.tools.contains(&"read_file".to_string()));
        assert_eq!(c.bash_commands.len(), 2, "both shell commands captured");
        assert_eq!(c.usage.cache_read_input_tokens, 200);
        assert_eq!(c.usage.input_tokens, 800, "uncached = 1000 - 200");
        assert!(c.cost_usd > 0.0);
    }

    #[test]
    fn read_turns_dedupes_consecutive_token_count_with_same_cumulative_total() {
        // codex 偶尔会重发上一帧的 token_count（同一个 total_tokens 出现两次）。
        // 我们必须只计一条 —— 否则 calls 会被多算，cost 会双倍。
        crate::stats::pricing::seed_test_prices();
        let p = write_temp(
            "codex-dup-token-count.jsonl",
            &[
                r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp"}}"#,
                r#"{"type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5"}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,"total_tokens":150},"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,"total_tokens":150}}}}"#,
                // 同样的 cumulative 重发一次 —— 必须跳过
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,"total_tokens":150},"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,"total_tokens":150}}}}"#,
            ],
        );
        let turns = read_turns(&p);
        let calls = &turns[0].calls;
        assert_eq!(calls.len(), 1, "expected dedup to drop the second event");
    }

    #[test]
    fn read_turns_uses_latest_turn_context_when_model_changes_mid_session() {
        // mid-session 切模型（gpt-5.3-codex → gpt-5.5），最后一条 turn_context 胜出。
        let p = write_temp(
            "codex-model-switch.jsonl",
            &[
                r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp","originator":"codex-tui"}}"#,
                r#"{"type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5.3-codex"}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"a"}}"#,
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"b"}}"#,
                r#"{"type":"turn_context","payload":{"turn_id":"t2","model":"gpt-5.5"}}"#,
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"c"}}"#,
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"d"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":500,"reasoning_output_tokens":0,"total_tokens":1500}}}}"#,
            ],
        );
        let turns = read_turns(&p);
        let last_call = turns.last().and_then(|t| t.calls.last()).expect("call");
        assert_eq!(last_call.model, "gpt-5.5");
    }

    #[test]
    fn read_session_keeps_commentary_and_final_answers_but_counts_only_visible_turns() {
        let p = write_temp(
            "codex-read-session-commentary.jsonl",
            &[
                r#"{"timestamp":"2026-06-08T02:12:13.012Z","type":"session_meta","payload":{"id":"abc","cwd":"/tmp"}}"#,
                r#"{"timestamp":"2026-06-08T02:12:15.000Z","type":"event_msg","payload":{"type":"user_message","message":"hi","images":[],"local_images":[],"text_elements":[]}}"#,
                r#"{"timestamp":"2026-06-08T02:12:16.000Z","type":"event_msg","payload":{"type":"agent_message","message":"checking...","phase":"commentary"}}"#,
                r#"{"timestamp":"2026-06-08T02:12:17.000Z","type":"event_msg","payload":{"type":"agent_message","message":"hello back","phase":"final_answer"}}"#,
                r#"{"timestamp":"2026-06-08T02:12:18.000Z","type":"event_msg","payload":{"type":"user_message","message":"vue3","images":[],"local_images":[],"text_elements":[]}}"#,
                r#"{"timestamp":"2026-06-08T02:12:19.000Z","type":"event_msg","payload":{"type":"agent_message","message":"scanning repo...","phase":"commentary"}}"#,
                r#"{"timestamp":"2026-06-08T02:12:20.000Z","type":"event_msg","payload":{"type":"agent_message","message":"What about Vue 3?","phase":"final_answer"}}"#,
            ],
        );

        let title_index = HashMap::new();
        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &title_index)
            .expect("session should parse");
        assert_eq!(
            msgs.len(),
            6,
            "commentary + final answers should all render"
        );
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].blocks[0].text.as_deref(), Some("hi"));
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].blocks[0].text.as_deref(), Some("checking..."));
        assert_eq!(msgs[2].role, "assistant");
        assert_eq!(msgs[2].blocks[0].text.as_deref(), Some("hello back"));
        assert_eq!(msgs[3].role, "user");
        assert_eq!(msgs[3].blocks[0].text.as_deref(), Some("vue3"));
        assert_eq!(msgs[4].role, "assistant");
        assert_eq!(msgs[4].blocks[0].text.as_deref(), Some("scanning repo..."));
        assert_eq!(msgs[5].role, "assistant");
        assert_eq!(msgs[5].blocks[0].text.as_deref(), Some("What about Vue 3?"));

        let meta = meta(&p).expect("meta");
        let session = scan(&p, &meta, &title_index, CodexThreadFlags::default());
        assert_eq!(
            session.message_count, 4,
            "commentary should not inflate session counts"
        );
    }

    #[test]
    fn read_session_renders_completed_plan_items() {
        let p = write_temp(
            "codex-read-session-plan-item.jsonl",
            &[
                r#"{"timestamp":"2026-07-30T10:25:49.000Z","type":"session_meta","payload":{"id":"abc","cwd":"/tmp"}}"#,
                r#"{"timestamp":"2026-07-30T10:25:50.000Z","type":"event_msg","payload":{"type":"user_message","message":"创建一个文件 plan-mode-test.txt，内容写 hello"}}"#,
                r##"{"timestamp":"2026-07-30T10:25:51.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"Plan","id":"plan-1","text":"# 创建测试文件\n\n## Summary\n\n在仓库根目录新增 `plan-mode-test.txt`，文件内容为 `hello`（末尾换行）。\n\n## Test Plan\n\n确认文件存在，且完整内容精确为 `hello`。\n"}}}"##,
            ],
        );

        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &HashMap::new())
            .expect("session should parse");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, "assistant");
        let text = msgs[1].blocks[0].text.as_deref().unwrap_or_default();
        assert!(text.contains("# 创建测试文件"));
        assert!(text.contains("## Summary"));
        assert!(text.contains("## Test Plan"));
    }

    #[test]
    fn read_session_synthesizes_rename_from_title_index_when_rollout_lacks_event() {
        let p = write_temp(
            "codex-read-session-synth-rename.jsonl",
            &[
                r#"{"timestamp":"2026-06-08T02:12:13.012Z","type":"session_meta","payload":{"id":"abc","timestamp":"2026-06-08T02:12:13.012Z","cwd":"/tmp"}}"#,
                r#"{"timestamp":"2026-06-08T02:12:15.000Z","type":"event_msg","payload":{"type":"user_message","message":"time","images":[],"local_images":[],"text_elements":[]}}"#,
                r#"{"timestamp":"2026-06-08T02:12:16.000Z","type":"event_msg","payload":{"type":"agent_message","message":"我取一下本机当前时间。","phase":"commentary"}}"#,
                r#"{"timestamp":"2026-06-08T02:12:17.000Z","type":"event_msg","payload":{"type":"agent_message","message":"2026-06-08 11:46:21 CST","phase":"final_answer"}}"#,
            ],
        );

        let mut title_index = HashMap::new();
        title_index.insert(
            "abc".to_string(),
            TitleIndexEntry {
                name: "codex-live".to_string(),
                updated_at_ms: parse_iso8601_ms("2026-06-08T04:43:09.162185Z"),
            },
        );

        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &title_index)
            .expect("session should parse");
        let rename = msgs
            .iter()
            .find(|m| {
                m.role == "user"
                    && m.blocks
                        .first()
                        .and_then(|b| b.text.as_deref())
                        .map(|text| text.contains("The user named this session \"codex-live\""))
                        .unwrap_or(false)
            })
            .expect("rename system event should be synthesized");
        assert_eq!(
            rename.timestamp.as_deref(),
            Some("2026-06-08T04:43:09.162Z")
        );
    }

    #[test]
    fn read_session_augments_apply_patch_with_patch_apply_end_changes() {
        let p = write_temp(
            "codex-read-session-apply-patch-delete.jsonl",
            &[
                r#"{"timestamp":"2026-06-08T05:30:56.167Z","type":"response_item","payload":{"type":"custom_tool_call","status":"completed","call_id":"call_patch_1","name":"apply_patch","input":"*** Begin Patch\n*** Delete File: /repo/src/old.ts\n*** Update File: /repo/test/new.test.ts\n@@\n+new line\n*** End Patch\n"}}"#,
                r#"{"timestamp":"2026-06-08T05:30:56.216Z","type":"event_msg","payload":{"type":"patch_apply_end","call_id":"call_patch_1","changes":{"/repo/src/old.ts":{"type":"delete","content":"alpha\nbeta\n"},"/repo/test/new.test.ts":{"type":"update","unified_diff":"@@ -1 +1,2 @@\n old line\n+new line","move_path":null}}}}"#,
                r#"{"timestamp":"2026-06-08T05:30:56.318Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call_patch_1","output":"ok"}}"#,
            ],
        );

        let title_index = HashMap::new();
        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &title_index)
            .expect("session should parse");
        let tool_use = msgs
            .iter()
            .flat_map(|m| m.blocks.iter())
            .find(|b| b.kind == "tool_use" && b.tool_name.as_deref() == Some("apply_patch"))
            .expect("apply_patch tool_use should exist");
        let input = tool_use.tool_input.as_deref().unwrap_or("");
        assert!(input.contains("*** Delete File: /repo/src/old.ts"));
        assert!(input.contains("@@"));
        assert!(input.contains("-alpha"));
        assert!(input.contains("-beta"));
        assert!(input.contains("*** Update File: /repo/test/new.test.ts"));
        assert!(input.contains("+new line"));
    }

    #[test]
    fn read_session_restores_apply_patch_wrapped_by_exec() {
        let exec_input = r#"const patch = "*** Begin Patch\n*** Add File: /repo/src/new.ts\n+export const value = 1;\n*** Update File: /repo/src/current.ts\n@@\n-old\n+new\n*** Delete File: /repo/src/old.ts\n*** End Patch";
const result = await tools.apply_patch(patch);
text(typeof result === 'string' ? result : JSON.stringify(result));"#;
        let lines = [
            json!({
                "timestamp": "2026-07-20T06:48:37.000Z",
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "call_id": "call_exec_patch",
                    "name": "exec",
                    "input": exec_input,
                },
            })
            .to_string(),
            json!({
                "timestamp": "2026-07-20T06:48:37.050Z",
                "type": "event_msg",
                "payload": {
                    "type": "patch_apply_end",
                    "call_id": "exec-internal-patch-id",
                    "changes": {
                        "/repo/src/new.ts": {
                            "type": "add",
                            "content": "export const value = 1;\n",
                        },
                        "/repo/src/current.ts": {
                            "type": "update",
                            "unified_diff": "@@ -10,2 +10,2 @@\n context\n-old\n+new\n",
                            "move_path": null,
                        },
                        "/repo/src/old.ts": {
                            "type": "delete",
                            "content": "old line\n",
                        },
                    },
                },
            })
            .to_string(),
            json!({
                "timestamp": "2026-07-20T06:48:37.100Z",
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call_output",
                    "call_id": "call_exec_patch",
                    "output": "{}",
                },
            })
            .to_string(),
        ];
        let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let p = write_temp("codex-read-session-exec-apply-patch.jsonl", &line_refs);

        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &HashMap::new())
            .expect("session should parse");
        let tool_use = msgs
            .iter()
            .flat_map(|m| m.blocks.iter())
            .find(|b| b.kind == "tool_use")
            .expect("apply_patch tool use should exist");

        assert_eq!(tool_use.tool_name.as_deref(), Some("apply_patch"));
        assert_eq!(tool_use.tool_id.as_deref(), Some("call_exec_patch"));
        let patch = tool_use.tool_input.as_deref().unwrap_or("");
        assert!(patch.contains("*** Add File: /repo/src/new.ts"));
        assert!(patch.contains("@@ -0,0 +1,1 @@"));
        assert!(patch.contains("+export const value = 1;"));
        assert!(patch.contains("*** Update File: /repo/src/current.ts"));
        assert!(patch.contains("@@ -10,2 +10,2 @@"));
        assert!(patch.contains("-old"));
        assert!(patch.contains("+new"));
        assert!(patch.contains("*** Delete File: /repo/src/old.ts"));
        assert!(patch.contains("@@ -1,1 +0,0 @@"));
        assert!(patch.contains("-old line"));
    }

    #[test]
    fn read_session_marks_failed_exec_apply_patch_as_error() {
        let exec_input = r#"const patch = "*** Begin Patch\n*** Update File: /repo/src/example.ts\n@@\n-old\n+new\n*** End Patch";
const result = await tools.apply_patch(patch);"#;
        let lines = [
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "call_id": "call_failed_patch",
                    "name": "exec",
                    "input": exec_input,
                },
            })
            .to_string(),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call_output",
                    "call_id": "call_failed_patch",
                    "output": [
                        { "type": "input_text", "text": "Script failed" },
                        { "type": "input_text", "text": "Script error: apply_patch verification failed" },
                    ],
                },
            })
            .to_string(),
        ];
        let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let p = write_temp(
            "codex-read-session-failed-exec-apply-patch.jsonl",
            &line_refs,
        );

        let msgs = read_with_title_index(p.to_string_lossy().as_ref(), &HashMap::new())
            .expect("session should parse");
        let tool_use = msgs
            .iter()
            .flat_map(|msg| msg.blocks.iter())
            .find(|block| block.kind == "tool_use")
            .expect("patch call should exist");
        assert_eq!(tool_use.tool_name.as_deref(), Some("apply_patch"));
        assert!(tool_use.is_error);
        assert!(msgs
            .iter()
            .flat_map(|msg| msg.blocks.iter())
            .any(|block| block.kind == "tool_result" && block.is_error));
    }

    #[test]
    #[ignore = "manual full-scan; reads every Codex rollout on disk"]
    fn dedup_full_codex_scan() {
        let src = CodexSource;
        let projects = src.list_projects(false, false).unwrap();
        let mut agg = crate::stats::aggregate::Aggregator::new();
        for p in &projects {
            let sessions = src.discover_stats_sessions(&p.dir_name).unwrap_or_default();
            for s in sessions {
                let turns = read_turns(std::path::Path::new(&s.path));
                agg.feed_session(&crate::stats::aggregate::SessionFeed {
                    agent: "codex",
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
        let s = agg.snapshot("codex");
        eprintln!("\n=== FULL CODEX SCAN ===");
        eprintln!("sessions: {}", s.session_count);
        eprintln!("calls: {}", s.call_count);
        eprintln!("cost: ${:.2}", s.cost_usd);
        eprintln!(
            "input: {} ({:.1}M)",
            s.usage.input_tokens,
            s.usage.input_tokens as f64 / 1e6
        );
        eprintln!(
            "output: {} ({:.1}M)",
            s.usage.output_tokens,
            s.usage.output_tokens as f64 / 1e6
        );
        eprintln!(
            "cache_read: {} ({:.1}M)",
            s.usage.cache_read_input_tokens,
            s.usage.cache_read_input_tokens as f64 / 1e6
        );
    }
}
#[test]
fn gui_chat_uses_codex_app_server_process_model() {
    assert_eq!(
        <CodexSource as crate::agents::SessionSource>::chat_process_model(&CodexSource),
        ChatProcessModel::CodexAppServer
    );
    assert_eq!(ChatProcessModel::CodexAppServer.as_str(), "codexAppServer");
}
