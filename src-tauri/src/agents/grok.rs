//! Grok CLI local-session source.
//!
//! A Grok session is directory-backed. `updates.jsonl` is the authoritative
//! user-visible event stream, while `summary.json` supplies list metadata.
//! `chat_history.jsonl` is model context and intentionally is not used as the
//! UI transcript.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use rayon::prelude::*;
use rusqlite::OpenFlags;
use serde_json::Value;

use super::{SessionSource, SessionStorageKind, SessionStorageUnit};
use crate::agent_command::AgentCommand;
use crate::stats::pricing;
use crate::stats::shell::{extract_first_command, extract_mcp_server};
use crate::stats::types::{CallRecord, Turn};
use crate::types::{Block, Msg, ProjectInfo, SessionMeta, SessionPage, UsageSummary};
use crate::util::{clean_title, home, mtime_millis, now_millis, text_block, validate_rename_name};

pub struct GrokSource;

const UPDATES_FILE: &str = "updates.jsonl";
const SUMMARY_FILE: &str = "summary.json";
const SUMMARY_LOCK_STALE_AFTER: Duration = Duration::from_secs(5 * 60);

pub fn config_path() -> PathBuf {
    grok_home().join("config.toml")
}

pub fn find_updates_path(session_id: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let expected_cwd = cwd.map(str::trim).filter(|cwd| !cwd.is_empty());
    discover_session_records(&grok_home(), true, true)
        .ok()?
        .into_iter()
        .find(|record| {
            record.id == session_id
                && expected_cwd.is_none_or(|expected_cwd| record.cwd == expected_cwd)
        })
        .map(|record| record.updates_path)
}

fn grok_home() -> PathBuf {
    let configured = std::env::var_os("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_grok_home(configured, &home(), &current_dir)
}

fn resolve_grok_home(
    configured: Option<PathBuf>,
    default_home: &Path,
    current_dir: &Path,
) -> PathBuf {
    let configured = configured.unwrap_or_else(|| default_home.join(".grok"));
    if configured.is_absolute() {
        configured
    } else {
        current_dir.join(configured)
    }
}

fn sessions_dir(root: &Path) -> PathBuf {
    root.join("sessions")
}

fn nonempty_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn percent_decode(input: &str) -> String {
    fn hex(value: u8) -> Option<u8> {
        match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            b'A'..=b'F' => Some(value - b'A' + 10),
            _ => None,
        }
    }

    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn fallback_cwd(group_dir: &Path) -> String {
    let cwd_marker = group_dir.join(".cwd");
    if let Ok(value) = fs::read_to_string(cwd_marker) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    group_dir
        .file_name()
        .map(|name| percent_decode(&name.to_string_lossy()))
        .unwrap_or_default()
}

fn parse_timestamp_ms(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
}

fn event_timestamp_ms(event: &Value) -> Option<i64> {
    event
        .pointer("/params/_meta/agentTimestampMs")
        .and_then(Value::as_i64)
        .or_else(|| {
            event.get("timestamp").and_then(Value::as_i64).map(|value| {
                if value.abs() < 10_000_000_000 {
                    value.saturating_mul(1000)
                } else {
                    value
                }
            })
        })
}

fn event_timestamp(event: &Value) -> Option<String> {
    let millis = event_timestamp_ms(event)?;
    Utc.timestamp_millis_opt(millis)
        .single()
        .map(|time| time.to_rfc3339())
}

fn read_summary(session_dir: &Path) -> Value {
    fs::read_to_string(session_dir.join(SUMMARY_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null)
}

fn summary_cwd(summary: &Value, group_dir: &Path) -> String {
    summary
        .pointer("/info/cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| fallback_cwd(group_dir))
}

fn summary_modified(summary: &Value, session_dir: &Path) -> u64 {
    ["updated_at", "last_active_at", "created_at"]
        .into_iter()
        .filter_map(|key| nonempty_string(summary, key).and_then(parse_timestamp_ms))
        .chain(
            [SUMMARY_FILE, UPDATES_FILE, "signals.json"]
                .into_iter()
                .map(|name| mtime_millis(&session_dir.join(name))),
        )
        .max()
        .unwrap_or(0)
}

fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| directory_size(&entry.path()))
        .sum()
}

#[derive(Clone)]
struct GrokSessionRecord {
    session_dir: PathBuf,
    updates_path: PathBuf,
    id: String,
    cwd: String,
    summary: Value,
    modified: u64,
    hidden: bool,
    subagent: bool,
}

fn discover_session_records(
    root: &Path,
    include_hidden: bool,
    include_subagents: bool,
) -> Result<Vec<GrokSessionRecord>, String> {
    let base = sessions_dir(root);
    if !base.exists() {
        return Ok(Vec::new());
    }
    let groups = fs::read_dir(&base)
        .map_err(|error| format!("Failed to read Grok Build sessions directory: {error}"))?;
    let mut records = Vec::new();
    for group in groups.flatten() {
        let Ok(group_type) = group.file_type() else {
            continue;
        };
        if !group_type.is_dir() || group_type.is_symlink() {
            continue;
        }
        let group_dir = group.path();
        let Ok(session_entries) = fs::read_dir(&group_dir) else {
            continue;
        };
        for entry in session_entries.flatten() {
            let Ok(entry_type) = entry.file_type() else {
                continue;
            };
            if !entry_type.is_dir() || entry_type.is_symlink() {
                continue;
            }
            let session_dir = entry.path();
            let updates_path = session_dir.join(UPDATES_FILE);
            if !updates_path.is_file() {
                continue;
            }
            let id = session_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let summary = read_summary(&session_dir);
            let hidden = summary
                .get("hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let session_kind = summary
                .get("session_kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let subagent = session_kind.contains("subagent") || session_kind.contains("sub-agent");
            if (!include_hidden && hidden) || (!include_subagents && subagent) {
                continue;
            }
            records.push(GrokSessionRecord {
                cwd: summary_cwd(&summary, &group_dir),
                modified: summary_modified(&summary, &session_dir),
                session_dir,
                updates_path,
                id,
                summary,
                hidden,
                subagent,
            });
        }
    }
    Ok(records)
}

fn image_src(value: &Value) -> Option<String> {
    let direct = value
        .get("url")
        .or_else(|| value.pointer("/image_url/url"))
        .or_else(|| value.pointer("/source/url"))
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty());
    if let Some(url) = direct {
        return Some(url.to_string());
    }
    let data = value
        .get("data")
        .or_else(|| value.pointer("/source/data"))
        .and_then(Value::as_str)?;
    let mime = value
        .get("mimeType")
        .or_else(|| value.get("mime_type"))
        .or_else(|| value.pointer("/source/media_type"))
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    if data.starts_with("data:") {
        Some(data.to_string())
    } else {
        Some(format!("data:{mime};base64,{data}"))
    }
}

fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn blocks_from_content(value: &Value) -> Vec<Block> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![text_block("text", text)]
            }
        }
        Value::Array(items) => items.iter().flat_map(blocks_from_content).collect(),
        Value::Object(object) => {
            let kind = object.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "text" | "input_text" | "output_text" => object
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(|text| vec![text_block("text", text)])
                    .unwrap_or_default(),
                "image" | "input_image" | "image_url" => image_src(value)
                    .map(|src| {
                        vec![Block {
                            kind: "image".to_string(),
                            image_src: Some(src),
                            ..Default::default()
                        }]
                    })
                    .unwrap_or_else(|| vec![text_block("text", &json_text(value))]),
                "file" | "resource" | "attachment" => {
                    let path = object
                        .get("path")
                        .or_else(|| object.get("filePath"))
                        .or_else(|| object.get("file_path"))
                        .or_else(|| object.get("uri"))
                        .or_else(|| object.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if path.is_empty() {
                        vec![text_block("text", &json_text(value))]
                    } else {
                        vec![Block {
                            kind: "file".to_string(),
                            text: Some(path.to_string()),
                            file_path: Some(path.to_string()),
                            ..Default::default()
                        }]
                    }
                }
                "content" => object
                    .get("content")
                    .map(blocks_from_content)
                    .unwrap_or_default(),
                _ => {
                    if let Some(text) = object.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            return vec![text_block("text", text)];
                        }
                    }
                    if let Some(content) = object.get("content") {
                        let nested = blocks_from_content(content);
                        if !nested.is_empty() {
                            return nested;
                        }
                    }
                    vec![text_block("text", &json_text(value))]
                }
            }
        }
        _ => vec![text_block("text", &json_text(value))],
    }
}

fn content_text(value: &Value) -> String {
    blocks_from_content(value)
        .into_iter()
        .filter_map(|block| block.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn event_prompt_id(event: &Value) -> Option<String> {
    event
        .pointer("/params/_meta/promptId")
        .or_else(|| event.pointer("/params/update/prompt_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        // Grok 本地持久化的用户消息没有外层 promptId；同一轮文字和附件靠 update
        // 内的递增 promptIndex 关联。把它作为稳定 key，才能合并为同一个用户消息。
        .or_else(|| {
            event
                .pointer("/params/update/_meta/promptIndex")
                .and_then(Value::as_u64)
                .map(|index| format!("grok-prompt-index:{index}"))
        })
}

fn is_session_update_event(event: &Value) -> bool {
    matches!(
        event.get("method").and_then(Value::as_str),
        Some("session/update" | "_x.ai/session/update")
    )
}

fn event_model(event: &Value) -> Option<String> {
    event
        .pointer("/params/_meta/modelId")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StreamKey {
    role: &'static str,
    block_kind: &'static str,
    prompt_id: Option<String>,
    stream_start_ms: Option<i64>,
}

#[derive(Clone, Debug)]
struct LastChunk {
    key: StreamKey,
    message_index: usize,
    block_index: usize,
}

fn append_chunk(
    messages: &mut Vec<Msg>,
    last_chunk: &mut Option<LastChunk>,
    event: &Value,
    role: &'static str,
    block_kind: &'static str,
    fallback_model: Option<&str>,
) {
    let content = event
        .pointer("/params/update/content")
        .unwrap_or(&Value::Null);
    let mut blocks = blocks_from_content(content);
    if blocks.is_empty() {
        return;
    }
    if block_kind == "thinking" {
        for block in &mut blocks {
            if block.kind == "text" {
                block.kind = "thinking".to_string();
            }
        }
    }
    let key = StreamKey {
        role,
        block_kind,
        prompt_id: event_prompt_id(event),
        stream_start_ms: event
            .pointer("/params/_meta/streamStartMs")
            .and_then(Value::as_i64),
    };
    let can_merge = key.prompt_id.is_some() || key.stream_start_ms.is_some();
    if can_merge {
        if let Some(previous) = last_chunk.as_ref().filter(|previous| {
            previous.message_index + 1 == messages.len()
                && previous.key.role == role
                && previous.key.prompt_id == key.prompt_id
        }) {
            if previous.key == key && blocks.len() == 1 {
                if let (Some(existing), Some(extra)) = (
                    messages
                        .get_mut(previous.message_index)
                        .and_then(|message| message.blocks.get_mut(previous.block_index)),
                    blocks[0].text.as_deref(),
                ) {
                    if existing.kind == blocks[0].kind {
                        existing
                            .text
                            .get_or_insert_with(String::new)
                            .push_str(extra);
                        return;
                    }
                }
            }
            // Grok 把一条带附件的用户输入依次写成 text / image / image chunk。它们
            // promptIndex 相同但 block 类型不同，不能像连续文本那样拼接；追加到同一条
            // Msg 后，ChatView 会将缩略图渲染在对应用户气泡的上方。
            if role == "user" && key.prompt_id.is_some() {
                if let Some(message) = messages.get_mut(previous.message_index) {
                    let block_index = message.blocks.len() + blocks.len() - 1;
                    message.blocks.append(&mut blocks);
                    *last_chunk = Some(LastChunk {
                        key,
                        message_index: previous.message_index,
                        block_index,
                    });
                    return;
                }
            }
        }
    }

    let block_index = blocks.len().saturating_sub(1);
    messages.push(Msg {
        uuid: event
            .pointer("/params/_meta/eventId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        role: role.to_string(),
        timestamp: event_timestamp(event),
        model: event_model(event).or_else(|| fallback_model.map(str::to_owned)),
        sidechain: false,
        blocks,
        meta_kind: None,
    });
    *last_chunk = Some(LastChunk {
        key,
        message_index: messages.len() - 1,
        block_index,
    });
}

fn tool_name(update: &Value) -> String {
    nonempty_string(update, "title")
        .or_else(|| {
            update
                .pointer("/_meta/x.ai~1tool/name")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            update
                .pointer("/_meta/x.ai~1tool/label")
                .and_then(Value::as_str)
        })
        .unwrap_or("tool")
        .to_string()
}

fn tool_input(update: &Value) -> String {
    update
        .get("rawInput")
        .map(json_text)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "{}".to_string())
}

fn preferred_output(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => (!text.is_empty()).then(|| text.clone()),
        Value::Array(items) => {
            let joined = items
                .iter()
                .filter_map(preferred_output)
                .collect::<Vec<_>>()
                .join("\n");
            (!joined.is_empty()).then_some(joined)
        }
        Value::Object(object) => {
            const PREFERRED_KEYS: &[&str] = &[
                "output",
                "stdout",
                "stderr",
                "content",
                "Content",
                "FileContent",
                "NotFound",
                "TodosUpdated",
                "description",
            ];
            let joined = PREFERRED_KEYS
                .iter()
                .filter_map(|key| object.get(*key).and_then(preferred_output))
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                Some(json_text(value))
            } else {
                Some(joined)
            }
        }
        _ => Some(json_text(value)),
    }
}

fn tool_output(update: &Value) -> String {
    let content = update
        .get("content")
        .map(content_text)
        .filter(|text| !text.trim().is_empty());
    content
        .or_else(|| update.get("rawOutput").and_then(preferred_output))
        .or_else(|| nonempty_string(update, "title").map(str::to_owned))
        .unwrap_or_else(|| {
            update
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("completed")
                .to_string()
        })
}

fn plan_text(update: &Value) -> Option<String> {
    let entries = update.get("entries")?.as_array()?;
    if entries.is_empty() {
        return None;
    }
    let body = entries
        .iter()
        .filter_map(|entry| {
            let content = entry.get("content").and_then(Value::as_str)?.trim();
            if content.is_empty() {
                return None;
            }
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            let marker = if matches!(status, "completed" | "done") {
                "x"
            } else {
                " "
            };
            Some(format!("- [{marker}] {content}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!body.is_empty()).then(|| format!("Plan\n\n{body}"))
}

fn retry_text(update: &Value) -> String {
    let attempt = update.get("attempt").and_then(Value::as_u64);
    let max = update.get("max_retries").and_then(Value::as_u64);
    let detail = ["message", "reason", "error_type", "type"]
        .into_iter()
        .find_map(|key| nonempty_string(update, key))
        .unwrap_or("Grok Build is retrying this turn");
    match (attempt, max) {
        (Some(attempt), Some(max)) => format!("Retry {attempt}/{max}: {detail}"),
        _ => detail.to_string(),
    }
}

fn failed_hook_text(update: &Value) -> Option<String> {
    let event_name = update
        .get("event_name")
        .and_then(Value::as_str)
        .unwrap_or("hook");
    let failed = update
        .get("runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|run| {
            !matches!(
                run.pointer("/status/status").and_then(Value::as_str),
                Some("completed" | "success")
            )
        })
        .map(|run| {
            let name = run.get("name").and_then(Value::as_str).unwrap_or("hook");
            let status = run
                .pointer("/status/status")
                .and_then(Value::as_str)
                .unwrap_or("failed");
            format!("{name}: {status}")
        })
        .collect::<Vec<_>>();
    (!failed.is_empty())
        .then(|| format!("Grok Build {event_name} hook failed\n{}", failed.join("\n")))
}

fn meta_message(event: &Value, meta_kind: &str, text: String) -> Msg {
    Msg {
        uuid: event
            .pointer("/params/_meta/eventId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        role: "user".to_string(),
        timestamp: event_timestamp(event),
        model: event_model(event),
        sidechain: false,
        blocks: vec![text_block("text", &text)],
        meta_kind: Some(meta_kind.to_string()),
    }
}

fn system_message(event: &Value, text: String) -> Msg {
    meta_message(event, "system", text)
}

fn system_note_message(event: &Value, text: String) -> Msg {
    meta_message(event, "meta", text)
}

fn session_recap_text(update: &Value) -> Option<String> {
    nonempty_string(update, "summary").map(str::to_owned)
}

fn task_completion(update: &Value) -> Option<(String, String, bool)> {
    let snapshot = update.get("task_snapshot")?;
    let task_id = nonempty_string(snapshot, "task_id")?.to_string();
    let output = nonempty_string(snapshot, "output")
        .unwrap_or("Background task completed")
        .to_string();
    let failed = snapshot
        .get("exit_code")
        .and_then(Value::as_i64)
        .is_some_and(|code| code != 0)
        || snapshot
            .get("signal")
            .is_some_and(|signal| !signal.is_null())
        || matches!(
            snapshot.get("completed").and_then(Value::as_bool),
            Some(false)
        );
    Some((task_id, output, failed))
}

fn upsert_tool_result(
    messages: &mut Vec<Msg>,
    tool_result_by_id: &mut HashMap<String, usize>,
    event: &Value,
    fallback_model: Option<&str>,
    id: Option<String>,
    result_block: Block,
) {
    if let Some(message_index) = id
        .as_deref()
        .and_then(|tool_id| tool_result_by_id.get(tool_id))
        .copied()
    {
        if let Some(message) = messages.get_mut(message_index) {
            message.timestamp = event_timestamp(event);
            message.blocks = vec![result_block];
        }
    } else {
        messages.push(Msg {
            uuid: event
                .pointer("/params/_meta/eventId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            role: "user".to_string(),
            timestamp: event_timestamp(event),
            model: event_model(event).or_else(|| fallback_model.map(str::to_owned)),
            sidechain: false,
            blocks: vec![result_block],
            meta_kind: None,
        });
        if let Some(id) = id {
            tool_result_by_id.insert(id, messages.len() - 1);
        }
    }
}

fn summary_model_for_updates(path: &Path) -> Option<String> {
    let summary = path.parent().map(read_summary).unwrap_or(Value::Null);
    nonempty_string(&summary, "current_model_id").map(str::to_owned)
}

fn read_updates(path: &Path) -> Result<Vec<Msg>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("Failed to open Grok Build session: {error}"))?;
    let fallback_model = summary_model_for_updates(path);
    let mut messages = Vec::new();
    let mut seen_event_ids = HashSet::new();
    let mut last_chunk: Option<LastChunk> = None;
    let mut tool_message_by_id: HashMap<String, usize> = HashMap::new();
    let mut tool_result_by_id: HashMap<String, usize> = HashMap::new();
    let mut plan_message_index: Option<usize> = None;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            // Grok can leave the final JSONL line incomplete while streaming.
            continue;
        };
        if !is_session_update_event(&event) {
            continue;
        }
        if let Some(event_id) = event
            .pointer("/params/_meta/eventId")
            .and_then(Value::as_str)
        {
            if !seen_event_ids.insert(event_id.to_string()) {
                continue;
            }
        }
        let update = event.pointer("/params/update").unwrap_or(&Value::Null);
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "user_message_chunk" => {
                append_chunk(&mut messages, &mut last_chunk, &event, "user", "text", None)
            }
            "agent_message_chunk" => append_chunk(
                &mut messages,
                &mut last_chunk,
                &event,
                "assistant",
                "text",
                fallback_model.as_deref(),
            ),
            "agent_thought_chunk" => append_chunk(
                &mut messages,
                &mut last_chunk,
                &event,
                "assistant",
                "thinking",
                fallback_model.as_deref(),
            ),
            "tool_call" => {
                last_chunk = None;
                let id = update
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                messages.push(Msg {
                    uuid: event
                        .pointer("/params/_meta/eventId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    role: "assistant".to_string(),
                    timestamp: event_timestamp(&event),
                    model: event_model(&event).or_else(|| fallback_model.clone()),
                    sidechain: false,
                    blocks: vec![Block {
                        kind: "tool_use".to_string(),
                        tool_name: Some(tool_name(update)),
                        tool_input: Some(tool_input(update)),
                        tool_id: id.clone(),
                        ..Default::default()
                    }],
                    meta_kind: None,
                });
                if let Some(id) = id {
                    tool_message_by_id.insert(id, messages.len() - 1);
                }
            }
            "tool_call_update" => {
                last_chunk = None;
                let status = update.get("status").and_then(Value::as_str).unwrap_or("");
                if !matches!(status, "completed" | "failed") {
                    continue;
                }
                let id = update
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let failed = status == "failed";
                if failed {
                    if let Some(message_index) = id
                        .as_deref()
                        .and_then(|tool_id| tool_message_by_id.get(tool_id))
                        .copied()
                    {
                        if let Some(block) = messages
                            .get_mut(message_index)
                            .and_then(|message| message.blocks.first_mut())
                        {
                            block.is_error = true;
                        }
                    }
                }
                let result_block = Block {
                    kind: "tool_result".to_string(),
                    text: Some(tool_output(update)),
                    tool_id: id.clone(),
                    is_error: failed,
                    ..Default::default()
                };
                upsert_tool_result(
                    &mut messages,
                    &mut tool_result_by_id,
                    &event,
                    fallback_model.as_deref(),
                    id,
                    result_block,
                );
            }
            "plan" => {
                last_chunk = None;
                let Some(text) = plan_text(update) else {
                    continue;
                };
                if let Some(index) = plan_message_index {
                    if let Some(message) = messages.get_mut(index) {
                        message.timestamp = event_timestamp(&event);
                        message.blocks = vec![text_block("text", &text)];
                    }
                } else {
                    messages.push(Msg {
                        uuid: event
                            .pointer("/params/_meta/eventId")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        role: "assistant".to_string(),
                        timestamp: event_timestamp(&event),
                        model: event_model(&event).or_else(|| fallback_model.clone()),
                        sidechain: false,
                        blocks: vec![text_block("text", &text)],
                        meta_kind: None,
                    });
                    plan_message_index = Some(messages.len() - 1);
                }
            }
            "retry_state" => {
                last_chunk = None;
                messages.push(system_note_message(&event, retry_text(update)));
            }
            "hook_execution" => {
                if let Some(text) = failed_hook_text(update) {
                    // 成功 hook 只是 Grok 在同一轮 text / image chunk 之间插入的内部
                    // 生命周期事件，不能打断 promptIndex 附件合并。失败 hook 才是需要
                    // 展示的独立系统注记。
                    last_chunk = None;
                    messages.push(system_note_message(&event, text));
                }
            }
            "session_recap" => {
                last_chunk = None;
                if let Some(text) = session_recap_text(update) {
                    // Grok 在会话收尾时写入的简要回顾属于系统注记，不是用户可见的
                    // 未知协议错误。使用已有的 `meta` 类型，使其与 Claude 的
                    // System note 使用同一套轻量折叠展示。
                    messages.push(system_note_message(&event, text));
                }
            }
            "task_backgrounded" => {
                // 对应 tool_call_update 已经记录了「转入后台」状态；不重复把协议载荷渲染成
                // 系统卡片。
                last_chunk = None;
            }
            "task_completed" => {
                last_chunk = None;
                if let Some((task_id, output, failed)) = task_completion(update) {
                    if failed {
                        if let Some(message_index) = tool_message_by_id.get(&task_id).copied() {
                            if let Some(block) = messages
                                .get_mut(message_index)
                                .and_then(|message| message.blocks.first_mut())
                            {
                                block.is_error = true;
                            }
                        }
                    }
                    upsert_tool_result(
                        &mut messages,
                        &mut tool_result_by_id,
                        &event,
                        fallback_model.as_deref(),
                        Some(task_id.clone()),
                        Block {
                            kind: "tool_result".to_string(),
                            text: Some(output),
                            tool_id: Some(task_id),
                            is_error: failed,
                            ..Default::default()
                        },
                    );
                }
            }
            "turn_completed" => {
                last_chunk = None;
            }
            "" => {}
            _ => {
                last_chunk = None;
                messages.push(system_message(
                    &event,
                    format!(
                        "Unsupported Grok Build update ({kind})\n{}",
                        json_text(update)
                    ),
                ));
            }
        }
    }
    crate::util::post_process_session_msgs(&mut messages);
    Ok(messages)
}

fn visible_message_count(messages: &[Msg]) -> usize {
    messages
        .iter()
        .filter(|message| {
            message.meta_kind.is_none()
                && message.blocks.iter().any(|block| {
                    matches!(block.kind.as_str(), "text" | "thinking" | "image" | "file")
                })
        })
        .count()
}

fn first_user_prompt(messages: &[Msg]) -> Option<String> {
    messages
        .iter()
        .filter(|message| message.role == "user" && message.meta_kind.is_none())
        .flat_map(|message| &message.blocks)
        .filter(|block| block.kind == "text")
        .filter_map(|block| block.text.as_deref())
        .map(clean_title)
        .find(|title| !title.is_empty())
}

fn session_title(record: &GrokSessionRecord, messages: &[Msg]) -> String {
    nonempty_string(&record.summary, "generated_title")
        .or_else(|| nonempty_string(&record.summary, "session_summary"))
        .map(clean_title)
        .filter(|title| !title.is_empty())
        .or_else(|| first_user_prompt(messages))
        .unwrap_or_else(|| record.id.clone())
}

fn session_meta(record: &GrokSessionRecord) -> SessionMeta {
    let messages = read_updates(&record.updates_path).unwrap_or_default();
    SessionMeta {
        id: record.id.clone(),
        file_name: UPDATES_FILE.to_string(),
        path: record.updates_path.to_string_lossy().to_string(),
        title: session_title(record, &messages),
        cwd: (!record.cwd.is_empty()).then(|| record.cwd.clone()),
        created: nonempty_string(&record.summary, "created_at").map(str::to_owned),
        modified: record.modified,
        size: directory_size(&record.session_dir),
        message_count: visible_message_count(&messages),
        codex_app_list_rank: None,
        codex_app_list_scanned: 0,
        codex_app_first_page_size: 0,
        codex_app_first_page_position: 0,
        codex_internal: false,
        codex_archived: false,
    }
}

fn usage_from_value(value: &Value) -> UsageSummary {
    let input = value
        .get("inputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = value
        .get("outputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = value
        .get("cachedReadTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning = value
        .get("reasoningTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    UsageSummary {
        input_tokens: input.saturating_sub(cache_read),
        output_tokens: output.saturating_sub(reasoning),
        cache_read_input_tokens: cache_read,
        reasoning_output_tokens: reasoning,
        ..Default::default()
    }
    .finalize()
}

fn usage_delta(current: UsageSummary, previous: UsageSummary) -> UsageSummary {
    UsageSummary {
        input_tokens: current.input_tokens.saturating_sub(previous.input_tokens),
        output_tokens: current.output_tokens.saturating_sub(previous.output_tokens),
        cache_creation_input_tokens: current
            .cache_creation_input_tokens
            .saturating_sub(previous.cache_creation_input_tokens),
        cache_creation_1h_input_tokens: current
            .cache_creation_1h_input_tokens
            .saturating_sub(previous.cache_creation_1h_input_tokens),
        cache_read_input_tokens: current
            .cache_read_input_tokens
            .saturating_sub(previous.cache_read_input_tokens),
        reasoning_output_tokens: current
            .reasoning_output_tokens
            .saturating_sub(previous.reasoning_output_tokens),
        total: 0,
    }
    .finalize()
}

fn latest_usage(path: &Path) -> Result<UsageSummary, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("Failed to open Grok Build session: {error}"))?;
    let mut latest = UsageSummary::default();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if event
            .pointer("/params/update/sessionUpdate")
            .and_then(Value::as_str)
            != Some("turn_completed")
        {
            continue;
        }
        if let Some(usage) = event.pointer("/params/update/usage") {
            latest = usage_from_value(usage);
        }
    }
    Ok(latest)
}

#[derive(Default)]
struct TurnExtras {
    tools: Vec<String>,
    bash_commands: Vec<String>,
    mcp_servers: Vec<String>,
}

fn ensure_turn(
    turns: &mut Vec<Turn>,
    extras: &mut Vec<TurnExtras>,
    by_prompt: &mut HashMap<String, usize>,
    prompt_id: String,
    session_id: &str,
    project_path: &str,
    timestamp_ms: i64,
) -> usize {
    if let Some(index) = by_prompt.get(&prompt_id).copied() {
        return index;
    }
    turns.push(Turn {
        project_path: project_path.to_string(),
        session_id: session_id.to_string(),
        timestamp_ms,
        ..Default::default()
    });
    extras.push(TurnExtras::default());
    let index = turns.len() - 1;
    by_prompt.insert(prompt_id, index);
    index
}

fn append_model_calls(
    turn: &mut Turn,
    model: &str,
    usage: UsageSummary,
    call_count: u64,
    event_id: Option<&str>,
) {
    let cost = pricing::cost_usd_grok(model, &usage);
    turn.calls.push(CallRecord {
        call_count: call_count.max(1),
        model: model.to_string(),
        message_id: event_id.map(|id| format!("grok:{id}:{model}")),
        usage,
        cost_usd: cost.map(|priced| priced.cost_usd).unwrap_or(0.0),
        pricing_missing: cost.is_none(),
        pricing_estimated: cost.is_some_and(|priced| priced.estimated),
        ..Default::default()
    });
}

fn read_turns_from_updates(path: &Path) -> Result<Vec<Turn>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("Failed to open Grok Build session: {error}"))?;
    let summary = path.parent().map(read_summary).unwrap_or(Value::Null);
    let fallback_model = nonempty_string(&summary, "current_model_id")
        .unwrap_or("grok")
        .to_string();
    let session_id = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let project_path = summary
        .pointer("/info/cwd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut turns = Vec::new();
    let mut extras = Vec::new();
    let mut by_prompt = HashMap::new();
    let mut current_prompt: Option<String> = None;
    let mut implicit_index = 0usize;
    let mut previous_usage = UsageSummary::default();
    let mut previous_calls = 0u64;
    let mut previous_models: HashMap<String, (UsageSummary, u64)> = HashMap::new();
    let mut seen_event_ids = HashSet::new();

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !is_session_update_event(&event) {
            continue;
        }
        if let Some(event_id) = event
            .pointer("/params/_meta/eventId")
            .and_then(Value::as_str)
        {
            if !seen_event_ids.insert(event_id.to_string()) {
                continue;
            }
        }
        let update = event.pointer("/params/update").unwrap_or(&Value::Null);
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        let prompt = event_prompt_id(&event).or_else(|| current_prompt.clone());
        let prompt = prompt.unwrap_or_else(|| {
            implicit_index += 1;
            format!("implicit-{implicit_index}")
        });
        let timestamp_ms = event_timestamp_ms(&event).unwrap_or(0);
        match kind {
            "user_message_chunk" => {
                current_prompt = Some(prompt.clone());
                let index = ensure_turn(
                    &mut turns,
                    &mut extras,
                    &mut by_prompt,
                    prompt,
                    &session_id,
                    &project_path,
                    timestamp_ms,
                );
                let text = update.get("content").map(content_text).unwrap_or_default();
                turns[index].user_message.push_str(&text);
            }
            "tool_call" => {
                let index = ensure_turn(
                    &mut turns,
                    &mut extras,
                    &mut by_prompt,
                    prompt,
                    &session_id,
                    &project_path,
                    timestamp_ms,
                );
                let name = tool_name(update);
                let input = tool_input(update);
                if let Some(command) = extract_first_command(&input) {
                    if matches!(
                        name.as_str(),
                        "run_terminal_command" | "execute" | "bash" | "shell"
                    ) {
                        extras[index].bash_commands.push(command);
                    }
                }
                if let Some(server) = extract_mcp_server(&name) {
                    extras[index].mcp_servers.push(server);
                }
                extras[index].tools.push(name);
            }
            "turn_completed" => {
                current_prompt = Some(prompt.clone());
                let index = ensure_turn(
                    &mut turns,
                    &mut extras,
                    &mut by_prompt,
                    prompt,
                    &session_id,
                    &project_path,
                    timestamp_ms,
                );
                let event_id = event
                    .pointer("/params/_meta/eventId")
                    .and_then(Value::as_str);
                let Some(usage_value) = update.get("usage") else {
                    continue;
                };
                let current_usage = usage_from_value(usage_value);
                let current_top_level_calls = usage_value
                    .get("modelCalls")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let model_usage = usage_value.get("modelUsage").and_then(Value::as_object);
                if let Some(models) = model_usage.filter(|models| !models.is_empty()) {
                    for (model, current_value) in models {
                        let current_model_usage = usage_from_value(current_value);
                        let current_calls = current_value
                            .get("modelCalls")
                            .and_then(Value::as_u64)
                            .unwrap_or(1);
                        let (previous_model_usage, previous_calls) =
                            previous_models.get(model).copied().unwrap_or_default();
                        let delta = usage_delta(current_model_usage, previous_model_usage);
                        let call_delta = current_calls.saturating_sub(previous_calls);
                        if delta.total > 0 || call_delta > 0 {
                            append_model_calls(
                                &mut turns[index],
                                model,
                                delta,
                                call_delta,
                                event_id,
                            );
                        }
                        previous_models.insert(model.clone(), (current_model_usage, current_calls));
                    }
                } else {
                    let delta = usage_delta(current_usage, previous_usage);
                    let call_delta = current_top_level_calls.saturating_sub(previous_calls);
                    if delta.total > 0 || call_delta > 0 {
                        append_model_calls(
                            &mut turns[index],
                            event_model(&event).as_deref().unwrap_or(&fallback_model),
                            delta,
                            call_delta,
                            event_id,
                        );
                    }
                }
                previous_usage = current_usage;
                previous_calls = current_top_level_calls;
            }
            _ => {}
        }
    }

    for (turn, extra) in turns.iter_mut().zip(extras) {
        if turn.calls.is_empty() && !extra.tools.is_empty() {
            turn.calls.push(CallRecord {
                // 工具事件可以先于/缺少 turn_completed；保留工具分析，但不能把这个
                // 合成 carrier 伪装成一次模型调用或一个有价格的模型记录。
                call_count: 0,
                ..Default::default()
            });
        }
        if let Some(call) = turn.calls.first_mut() {
            call.tools = extra.tools;
            call.bash_commands = extra.bash_commands;
            call.mcp_servers = extra.mcp_servers;
        }
    }
    Ok(turns)
}

fn source_mtime(path: &Path) -> u64 {
    let Some(session_dir) = path.parent() else {
        return mtime_millis(path);
    };
    [UPDATES_FILE, SUMMARY_FILE, "signals.json"]
        .into_iter()
        .map(|name| mtime_millis(&session_dir.join(name)))
        .max()
        .unwrap_or(0)
}

fn canonical_sessions_root(root: &Path) -> Result<PathBuf, String> {
    sessions_dir(root)
        .canonicalize()
        .map_err(|error| format!("Failed to resolve Grok Build sessions directory: {error}"))
}

fn validate_existing_storage(
    root: &Path,
    updates_path: &Path,
) -> Result<SessionStorageUnit, String> {
    if updates_path.file_name().and_then(|name| name.to_str()) != Some(UPDATES_FILE) {
        return Err("Grok Build session path must point to updates.jsonl".to_string());
    }
    if !updates_path.is_file() {
        return Err("Grok Build updates.jsonl does not exist".to_string());
    }
    let requested_session_root = updates_path
        .parent()
        .ok_or_else(|| "Invalid Grok Build session directory".to_string())?;
    let requested_group_root = requested_session_root
        .parent()
        .ok_or_else(|| "Invalid Grok Build project group directory".to_string())?;
    for (path, label) in [
        (requested_group_root, "project group"),
        (requested_session_root, "session directory"),
    ] {
        if fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(true)
        {
            return Err(format!("Grok Build {label} cannot be a symlink"));
        }
    }
    let sessions_root = canonical_sessions_root(root)?;
    let canonical_updates = updates_path
        .canonicalize()
        .map_err(|error| format!("Failed to resolve Grok Build session path: {error}"))?;
    let session_root = canonical_updates
        .parent()
        .ok_or_else(|| "Invalid Grok Build session directory".to_string())?
        .to_path_buf();
    let group_root = session_root
        .parent()
        .ok_or_else(|| "Invalid Grok Build project group directory".to_string())?;
    if group_root.parent() != Some(sessions_root.as_path()) {
        return Err("Grok Build session path is outside GROK_HOME/sessions".to_string());
    }
    Ok(SessionStorageUnit {
        root_path: session_root,
        entry_relative_path: PathBuf::from(UPDATES_FILE),
        kind: SessionStorageKind::Directory,
    })
}

fn validate_restore_storage(
    root: &Path,
    entry_path: &Path,
    session_root: &Path,
) -> Result<(), String> {
    let sessions_root = sessions_dir(root)
        .canonicalize()
        .unwrap_or_else(|_| sessions_dir(root));
    if entry_path != session_root.join(UPDATES_FILE) {
        return Err(
            "Grok Build restore entry must be updates.jsonl inside the session directory"
                .to_string(),
        );
    }
    let relative = session_root
        .strip_prefix(&sessions_root)
        .map_err(|_| "Grok Build restore target is outside GROK_HOME/sessions".to_string())?;
    let components: Vec<Component<'_>> = relative.components().collect();
    if components.len() != 2
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Invalid Grok Build restore directory structure".to_string());
    }
    if let Some(group) = session_root.parent().filter(|group| group.exists()) {
        if fs::symlink_metadata(group)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(true)
        {
            return Err("Grok Build restore project group cannot be a symlink".to_string());
        }
        let canonical_group = group
            .canonicalize()
            .map_err(|error| format!("Failed to resolve Grok Build project group: {error}"))?;
        if canonical_group.parent() != Some(sessions_root.as_path()) {
            return Err("Grok Build restore project group escapes GROK_HOME/sessions".to_string());
        }
    }
    Ok(())
}

struct SummaryLock {
    path: PathBuf,
}

impl Drop for SummaryLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn is_stale_empty_summary_lock(metadata: &fs::Metadata, now: std::time::SystemTime) -> bool {
    metadata.len() == 0
        && metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= SUMMARY_LOCK_STALE_AFTER)
}

fn stale_summary_lock(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| is_stale_empty_summary_lock(&metadata, std::time::SystemTime::now()))
        .unwrap_or(false)
}

fn acquire_summary_lock(session_dir: &Path) -> Result<SummaryLock, String> {
    let path = session_dir.join("summary.json.lock");
    let mut file = loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break file,
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && stale_summary_lock(&path) =>
            {
                // A crashed Grok process can leave an empty lock indefinitely.
                // Remove only an old empty lock; active writes remain protected.
                if fs::remove_file(&path).is_ok() {
                    continue;
                }
                return Err(
                    "Grok Build session metadata is currently locked; try again after the active write finishes"
                        .to_string(),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(
                    "Grok Build session metadata is currently locked; try again after the active write finishes"
                        .to_string(),
                );
            }
            Err(error) => {
                return Err(format!(
                    "Failed to lock Grok Build session metadata: {error}"
                ));
            }
        }
    };
    let _ = writeln!(file, "viewer-pid={}", std::process::id());
    let _ = file.sync_all();
    Ok(SummaryLock { path })
}

fn write_json_atomically(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Summary path has no parent directory".to_string())?;
    let temp = parent.join(format!(
        ".{SUMMARY_FILE}.viewer-{}-{}.tmp",
        std::process::id(),
        now_millis()
    ));
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Failed to serialize Grok Build summary: {error}"))?;
    bytes.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| format!("Failed to create Grok Build summary temp file: {error}"))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(format!("Failed to write Grok Build summary: {error}"));
    }
    drop(file);
    if let Err(first_error) = fs::rename(&temp, path) {
        // std::fs::rename does not replace an existing destination on Windows.
        // Preserve the original in a same-directory backup until the new file
        // has been installed successfully.
        let backup = parent.join(format!(
            ".{SUMMARY_FILE}.viewer-{}-{}.bak",
            std::process::id(),
            now_millis()
        ));
        if !path.exists() || fs::rename(path, &backup).is_err() {
            let _ = fs::remove_file(&temp);
            return Err(format!(
                "Failed to replace Grok Build summary: {first_error}"
            ));
        }
        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&temp);
            return Err(format!("Failed to install Grok Build summary: {error}"));
        }
        let _ = fs::remove_file(backup);
    }
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn sync_search_index_title(session_dir: &Path, session_id: &str, title: &str) {
    let Some(grok_root) = session_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return;
    };
    let database = grok_root.join("session_search.sqlite");
    if !database.is_file() {
        return;
    }
    let Ok(connection) = rusqlite::Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };
    let _ = connection.busy_timeout(Duration::from_millis(100));
    let _ = connection.execute(
        "UPDATE session_docs SET title = ?1 WHERE session_id = ?2",
        rusqlite::params![title, session_id],
    );
}

fn rename_session_at(root: &Path, path: &Path, name: &str) -> Result<(), String> {
    let title = validate_rename_name(name)?;
    let unit = validate_existing_storage(root, path)?;
    let _lock = acquire_summary_lock(&unit.root_path)?;
    let summary_path = unit.root_path.join(SUMMARY_FILE);
    let mut summary = read_summary(&unit.root_path);
    if !summary.is_object() {
        summary = serde_json::json!({
            "info": {
                "id": unit.root_path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default(),
                "cwd": unit.root_path.parent().map(fallback_cwd).unwrap_or_default(),
            }
        });
    }
    let object = summary
        .as_object_mut()
        .ok_or_else(|| "Grok Build summary root must be a JSON object".to_string())?;
    object.insert(
        "generated_title".to_string(),
        Value::String(title.to_string()),
    );
    object.insert("title_is_manual".to_string(), Value::Bool(true));
    write_json_atomically(&summary_path, &summary)?;
    let session_id = unit
        .root_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    sync_search_index_title(&unit.root_path, &session_id, title);
    Ok(())
}

impl SessionSource for GrokSource {
    fn name(&self) -> &'static str {
        "grok"
    }

    fn list_projects(
        &self,
        _include_codex_internal: bool,
        _include_codex_archived: bool,
    ) -> Result<Vec<ProjectInfo>, String> {
        let records = discover_session_records(&grok_home(), false, false)?;
        let mut projects: HashMap<String, ProjectInfo> = HashMap::new();
        for record in records {
            let entry = projects
                .entry(record.cwd.clone())
                .or_insert_with(|| ProjectInfo {
                    dir_name: record.cwd.clone(),
                    display_path: record.cwd.clone(),
                    session_count: 0,
                    last_modified: 0,
                    exists: Path::new(&record.cwd).is_dir(),
                    bookmarked: false,
                    parent_dir_name: None,
                    worktree_name: None,
                });
            entry.session_count += 1;
            entry.last_modified = entry.last_modified.max(record.modified);
        }
        let mut projects: Vec<ProjectInfo> = projects.into_values().collect();
        projects.sort_by_key(|project| std::cmp::Reverse(project.last_modified));
        Ok(projects)
    }

    fn list_sessions(
        &self,
        project_key: &str,
        offset: usize,
        limit: usize,
        _include_codex_internal: bool,
        _include_codex_archived: bool,
    ) -> Result<SessionPage, String> {
        let mut records: Vec<GrokSessionRecord> =
            discover_session_records(&grok_home(), false, false)?
                .into_iter()
                .filter(|record| record.cwd == project_key)
                .collect();
        records.sort_by_key(|record| std::cmp::Reverse(record.modified));
        let total = records.len();
        let window: Vec<&GrokSessionRecord> = records.iter().skip(offset).take(limit).collect();
        let sessions = window
            .par_iter()
            .map(|record| session_meta(record))
            .collect();
        Ok(SessionPage { total, sessions })
    }

    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String> {
        read_updates(Path::new(path))
    }

    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String> {
        rename_session_at(&grok_home(), path, name)
    }

    fn trash_title(&self, path: &Path) -> String {
        let session_dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let updates_path = session_dir.join(UPDATES_FILE);
        let summary = read_summary(session_dir);
        nonempty_string(&summary, "generated_title")
            .or_else(|| nonempty_string(&summary, "session_summary"))
            .map(clean_title)
            .filter(|title| !title.is_empty())
            .or_else(|| {
                read_updates(&updates_path)
                    .ok()
                    .and_then(|messages| first_user_prompt(&messages))
            })
            .unwrap_or_else(|| {
                session_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Grok Build session".to_string())
            })
    }

    fn resume_command(&self, session_id: &str, _path: &str) -> AgentCommand {
        AgentCommand::new("grok").arg("--resume").arg(session_id)
    }

    fn new_session_command(&self) -> AgentCommand {
        AgentCommand::new("grok")
    }

    fn image_src(&self, block: &Value) -> Option<String> {
        image_src(block)
    }

    fn usage_summary(&self, path: &str) -> Result<UsageSummary, String> {
        latest_usage(Path::new(path))
    }

    fn context_usage(&self, path: &str) -> Result<UsageSummary, String> {
        let signals_path = Path::new(path)
            .parent()
            .map(|parent| parent.join("signals.json"));
        let used = signals_path
            .and_then(|signals_path| fs::read_to_string(signals_path).ok())
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|signals| signals.get("contextTokensUsed").and_then(Value::as_u64))
            .unwrap_or(0);
        Ok(UsageSummary {
            input_tokens: used,
            ..Default::default()
        }
        .finalize())
    }

    fn last_prompt(&self, path: &str) -> Result<Option<String>, String> {
        let messages = read_updates(Path::new(path))?;
        Ok(messages
            .iter()
            .rev()
            .filter(|message| message.role == "user" && message.meta_kind.is_none())
            .flat_map(|message| message.blocks.iter())
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.as_deref())
            .map(crate::util::truncate_subtitle)
            .find(|text| !text.is_empty()))
    }

    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String> {
        read_turns_from_updates(Path::new(path))
    }

    fn discover_stats_sessions(&self, project_key: &str) -> Result<Vec<SessionMeta>, String> {
        let records: Vec<GrokSessionRecord> = discover_session_records(&grok_home(), true, true)?
            .into_iter()
            .filter(|record| record.cwd == project_key && (record.subagent || !record.hidden))
            .collect();
        Ok(records.par_iter().map(session_meta).collect())
    }

    fn source_mtime(&self, path: &str) -> u64 {
        source_mtime(Path::new(path))
    }

    fn validate_session_path(&self, path: &Path) -> Result<(), String> {
        validate_existing_storage(&grok_home(), path).map(|_| ())
    }

    fn session_storage_unit(&self, path: &Path) -> Result<SessionStorageUnit, String> {
        validate_existing_storage(&grok_home(), path)
    }

    fn validate_restore_target(
        &self,
        entry_path: &Path,
        root_path: &Path,
        kind: SessionStorageKind,
    ) -> Result<(), String> {
        if kind != SessionStorageKind::Directory {
            return Err("Grok Build sessions must be restored as directories".to_string());
        }
        validate_restore_storage(&grok_home(), entry_path, root_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "grok-source-test-{name}-{}-{}",
            std::process::id(),
            now_millis()
        ))
    }

    fn create_session(
        root: &Path,
        encoded_cwd: &str,
        id: &str,
        summary: Value,
        lines: &[Value],
    ) -> PathBuf {
        let session = root.join("sessions").join(encoded_cwd).join(id);
        fs::create_dir_all(&session).unwrap();
        fs::write(
            session.join(SUMMARY_FILE),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
        let body = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(session.join(UPDATES_FILE), body).unwrap();
        session.join(UPDATES_FILE)
    }

    fn event(id: &str, prompt: &str, kind: &str, payload: Value) -> Value {
        let mut update = payload.as_object().cloned().unwrap_or_default();
        update.insert("sessionUpdate".to_string(), Value::String(kind.to_string()));
        serde_json::json!({
            "timestamp": 1_700_000_000,
            "method": "session/update",
            "params": {
                "sessionId": "session-a",
                "update": update,
                "_meta": {
                    "eventId": id,
                    "promptId": prompt,
                    "streamStartMs": 1_700_000_000_000i64,
                    "agentTimestampMs": 1_700_000_000_000i64
                }
            }
        })
    }

    #[test]
    fn percent_decodes_project_directory() {
        assert_eq!(
            percent_decode("%2FUsers%2Fdemo%2F%E9%A1%B9%E7%9B%AE"),
            "/Users/demo/项目"
        );
    }

    #[test]
    fn resolves_default_absolute_and_relative_grok_home() {
        let base = scratch("home-resolver");
        let default_home = base.join("home");
        let current_dir = base.join("cwd");
        let absolute = base.join("absolute-grok");

        assert_eq!(
            resolve_grok_home(None, &default_home, &current_dir),
            default_home.join(".grok")
        );
        assert_eq!(
            resolve_grok_home(Some(absolute.clone()), &default_home, &current_dir),
            absolute
        );
        assert_eq!(
            resolve_grok_home(
                Some(PathBuf::from("relative-grok")),
                &default_home,
                &current_dir,
            ),
            current_dir.join("relative-grok")
        );
    }

    #[test]
    fn discovers_projects_and_uses_summary_metadata() {
        let root = scratch("discover");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({
                "info": {"id":"session-a","cwd":"/tmp/demo"},
                "generated_title":"Manual title",
                "created_at":"2026-08-18T01:02:03Z",
                "updated_at":"2026-08-18T02:03:04Z",
                "current_model_id":"grok-test"
            }),
            &[
                event(
                    "u1",
                    "p1",
                    "user_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"hello"}}),
                ),
                event(
                    "a1",
                    "p1",
                    "agent_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"world"}}),
                ),
            ],
        );
        let records = discover_session_records(&root, false, false).unwrap();
        assert_eq!(records.len(), 1);
        let meta = session_meta(&records[0]);
        assert_eq!(meta.title, "Manual title");
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/demo"));
        assert_eq!(meta.path, updates.to_string_lossy());
        assert_eq!(meta.message_count, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_dot_cwd_and_first_user_prompt() {
        let root = scratch("fallback");
        let group = root.join("sessions").join("long-slug-hash");
        fs::create_dir_all(&group).unwrap();
        fs::write(group.join(".cwd"), "/tmp/a very long project\n").unwrap();
        let updates = create_session(
            &root,
            "long-slug-hash",
            "session-a",
            Value::Null,
            &[event(
                "u1",
                "p1",
                "user_message_chunk",
                serde_json::json!({"content":{"type":"text","text":"  First real prompt  "}}),
            )],
        );
        let records = discover_session_records(&root, false, false).unwrap();
        let meta = session_meta(&records[0]);
        assert_eq!(meta.title, "First real prompt");
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/a very long project"));
        assert_eq!(meta.path, updates.to_string_lossy());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_and_corrupted_summaries_still_discover_sessions() {
        let root = scratch("broken-summary");
        let group = root.join("sessions").join("long-slug-hash");
        fs::create_dir_all(&group).unwrap();
        fs::write(group.join(".cwd"), "/tmp/fallback-project\n").unwrap();
        let missing = create_session(
            &root,
            "long-slug-hash",
            "missing-summary",
            Value::Null,
            &[event(
                "missing-user",
                "missing-prompt",
                "user_message_chunk",
                serde_json::json!({"content":{"type":"text","text":"Missing title fallback"}}),
            )],
        );
        fs::remove_file(missing.parent().unwrap().join(SUMMARY_FILE)).unwrap();
        let corrupted = create_session(
            &root,
            "long-slug-hash",
            "corrupted-summary",
            Value::Null,
            &[event(
                "corrupt-user",
                "corrupt-prompt",
                "user_message_chunk",
                serde_json::json!({"content":{"type":"text","text":"Corrupt title fallback"}}),
            )],
        );
        fs::write(corrupted.parent().unwrap().join(SUMMARY_FILE), "{broken").unwrap();

        let records = discover_session_records(&root, false, false).unwrap();
        assert_eq!(records.len(), 2);
        let metadata = records
            .iter()
            .map(|record| (record.id.as_str(), session_meta(record)))
            .collect::<HashMap<_, _>>();
        assert_eq!(metadata["missing-summary"].title, "Missing title fallback");
        assert_eq!(
            metadata["corrupted-summary"].title,
            "Corrupt title fallback"
        );
        assert!(metadata
            .values()
            .all(|meta| meta.cwd.as_deref() == Some("/tmp/fallback-project")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normal_discovery_filters_hidden_and_subagent_sessions() {
        let root = scratch("visibility");
        for (id, extra) in [
            ("visible", serde_json::json!({})),
            ("hidden", serde_json::json!({"hidden":true})),
            ("subagent", serde_json::json!({"session_kind":"subagent"})),
        ] {
            let mut summary = serde_json::json!({"info":{"id":id,"cwd":"/tmp/demo"}});
            summary
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            create_session(&root, "%2Ftmp%2Fdemo", id, summary, &[]);
        }

        let normal = discover_session_records(&root, false, false).unwrap();
        assert_eq!(normal.len(), 1);
        assert_eq!(normal[0].id, "visible");
        assert_eq!(
            discover_session_records(&root, true, false).unwrap().len(),
            2
        );
        assert_eq!(
            discover_session_records(&root, false, true).unwrap().len(),
            2
        );
        assert_eq!(
            discover_session_records(&root, true, true).unwrap().len(),
            3
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parser_merges_chunks_deduplicates_events_and_pairs_tools() {
        let root = scratch("parser");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"},"current_model_id":"grok-test"}),
            &[
                event(
                    "u1",
                    "p1",
                    "user_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"hel"}}),
                ),
                event(
                    "u2",
                    "p1",
                    "user_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"lo"}}),
                ),
                event(
                    "u2",
                    "p1",
                    "user_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"duplicate"}}),
                ),
                event(
                    "t1",
                    "p1",
                    "agent_thought_chunk",
                    serde_json::json!({"content":{"type":"text","text":"think"}}),
                ),
                event(
                    "c1",
                    "p1",
                    "tool_call",
                    serde_json::json!({"toolCallId":"tool-1","title":"read_file","rawInput":{"path":"src/main.rs"}}),
                ),
                event(
                    "r1",
                    "p1",
                    "tool_call_update",
                    serde_json::json!({"toolCallId":"tool-1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"file body"}}]}),
                ),
                event(
                    "a1",
                    "p1",
                    "agent_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"done"}}),
                ),
            ],
        );
        fs::write(
            updates.parent().unwrap().join(UPDATES_FILE),
            fs::read_to_string(&updates).unwrap() + "{partial",
        )
        .unwrap();
        let messages = read_updates(&updates).unwrap();
        assert_eq!(messages[0].blocks[0].text.as_deref(), Some("hello"));
        assert!(messages
            .iter()
            .any(|message| message.blocks.iter().any(|block| block.kind == "thinking")));
        let call = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .find(|block| block.kind == "tool_use")
            .unwrap();
        assert_eq!(call.tool_id.as_deref(), Some("tool-1"));
        let result = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .find(|block| block.kind == "tool_result")
            .unwrap();
        assert_eq!(result.tool_id.as_deref(), Some("tool-1"));
        assert_eq!(result.text.as_deref(), Some("file body"));
        assert!(!result.is_error);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parser_groups_grok_prompt_index_attachments_with_their_text() {
        fn user_chunk(id: &str, prompt_index: u64, content: Value) -> Value {
            let mut value = event(
                id,
                "ignored",
                "user_message_chunk",
                serde_json::json!({
                    "content": content,
                    "_meta": {"promptIndex": prompt_index, "modelId": "grok-test"}
                }),
            );
            let meta = value
                .pointer_mut("/params/_meta")
                .unwrap()
                .as_object_mut()
                .unwrap();
            meta.remove("promptId");
            meta.remove("streamStartMs");
            value
        }

        let root = scratch("prompt-index-attachments");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[
                user_chunk(
                    "text",
                    7,
                    serde_json::json!({"type":"text","text":"Describe these images"}),
                ),
                event(
                    "hook-after-text",
                    "internal",
                    "hook_execution",
                    serde_json::json!({
                        "event_name":"user_prompt_submit",
                        "runs":[{"name":"ok","status":{"status":"success"}}]
                    }),
                ),
                user_chunk(
                    "image-one",
                    7,
                    serde_json::json!({"type":"image","data":"AQID","mimeType":"image/png"}),
                ),
                event(
                    "hook-between-images",
                    "internal",
                    "hook_execution",
                    serde_json::json!({
                        "event_name":"pre_tool_use",
                        "runs":[{"name":"ok","status":{"status":"success"}}]
                    }),
                ),
                user_chunk(
                    "image-two",
                    7,
                    serde_json::json!({"type":"image","data":"BAUG","mimeType":"image/png"}),
                ),
                user_chunk(
                    "next-prompt",
                    8,
                    serde_json::json!({"type":"text","text":"A separate prompt"}),
                ),
            ],
        );

        let messages = read_updates(&updates).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].blocks.len(), 3);
        assert_eq!(
            messages[0].blocks[0].text.as_deref(),
            Some("Describe these images")
        );
        assert_eq!(messages[0].blocks[1].kind, "image");
        assert_eq!(messages[0].blocks[2].kind, "image");
        assert_eq!(
            messages[1].blocks[0].text.as_deref(),
            Some("A separate prompt")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parser_marks_failed_tool_call_and_result() {
        let root = scratch("failed-tool");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[
                event(
                    "call",
                    "prompt",
                    "tool_call",
                    serde_json::json!({"toolCallId":"tool-1","title":"shell","rawInput":{"command":"false"}}),
                ),
                event(
                    "failed",
                    "prompt",
                    "tool_call_update",
                    serde_json::json!({"toolCallId":"tool-1","status":"failed","rawOutput":{"stderr":"command failed"}}),
                ),
            ],
        );
        let messages = read_updates(&updates).unwrap();
        let tool_use = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .find(|block| block.kind == "tool_use")
            .unwrap();
        let tool_result = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .find(|block| block.kind == "tool_result")
            .unwrap();
        assert!(tool_use.is_error);
        assert!(tool_result.is_error);
        assert_eq!(tool_result.text.as_deref(), Some("command failed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parser_keeps_plan_retry_failures_and_unknown_content_without_hook_noise() {
        let root = scratch("meta-events");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[
                event(
                    "unknown-content",
                    "prompt",
                    "user_message_chunk",
                    serde_json::json!({"content":{"type":"future_content","payload":{"keep":true}}}),
                ),
                event(
                    "plan",
                    "prompt",
                    "plan",
                    serde_json::json!({"entries":[{"content":"Inspect","status":"completed"},{"content":"Implement","status":"pending"}]}),
                ),
                event(
                    "retry",
                    "prompt",
                    "retry_state",
                    serde_json::json!({"attempt":2,"max_retries":3,"reason":"temporary failure"}),
                ),
                event(
                    "hook-ok",
                    "prompt",
                    "hook_execution",
                    serde_json::json!({"event_name":"Stop","runs":[{"name":"ok","status":{"status":"completed"}}]}),
                ),
                event(
                    "hook-failed",
                    "prompt",
                    "hook_execution",
                    serde_json::json!({"event_name":"Stop","runs":[{"name":"broken","status":{"status":"failed"}}]}),
                ),
                event(
                    "future-update",
                    "prompt",
                    "future_update",
                    serde_json::json!({"detail":"preserve me"}),
                ),
            ],
        );
        let messages = read_updates(&updates).unwrap();
        let rendered = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .filter_map(|block| block.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("future_content"));
        assert!(rendered.contains("- [x] Inspect"));
        assert!(rendered.contains("- [ ] Implement"));
        assert!(rendered.contains("Retry 2/3: temporary failure"));
        assert!(rendered.contains("broken: failed"));
        assert!(!rendered.contains("ok: completed"));
        assert!(rendered.contains("Unsupported Grok Build update (future_update)"));
        assert!(rendered.contains("preserve me"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parser_renders_session_recap_as_a_system_note() {
        let root = scratch("session-recap");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[event(
                "recap",
                "prompt",
                "session_recap",
                serde_json::json!({
                    "summary":"The requested Markdown examples were rendered.",
                    "auto":true
                }),
            )],
        );

        let messages = read_updates(&updates).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].meta_kind.as_deref(), Some("meta"));
        assert_eq!(
            messages[0].blocks[0].text.as_deref(),
            Some("The requested Markdown examples were rendered.")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parser_keeps_background_task_protocol_out_of_system_notes() {
        let root = scratch("background-task");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[
                event(
                    "call",
                    "prompt",
                    "tool_call",
                    serde_json::json!({
                        "toolCallId":"task-1",
                        "title":"run_terminal_command",
                        "rawInput":{"command":"long-running-command"}
                    }),
                ),
                event(
                    "started",
                    "prompt",
                    "tool_call_update",
                    serde_json::json!({
                        "toolCallId":"task-1",
                        "status":"completed",
                        "content":[{"type":"content","content":{"type":"text","text":"Moved to background"}}]
                    }),
                ),
                event(
                    "backgrounded",
                    "prompt",
                    "task_backgrounded",
                    serde_json::json!({"task_id":"task-1"}),
                ),
                event(
                    "completed",
                    "prompt",
                    "task_completed",
                    serde_json::json!({
                        "task_snapshot":{
                            "task_id":"task-1",
                            "output":"final command output",
                            "exit_code":0,
                            "completed":true
                        }
                    }),
                ),
                event(
                    "retry",
                    "prompt",
                    "retry_state",
                    serde_json::json!({"reason":"temporary failure"}),
                ),
            ],
        );

        let messages = read_updates(&updates).unwrap();
        assert_eq!(messages.len(), 3);
        let result = messages[1].blocks.first().unwrap();
        assert_eq!(result.kind, "tool_result");
        assert_eq!(result.text.as_deref(), Some("final command output"));
        assert_eq!(result.tool_id.as_deref(), Some("task-1"));
        assert!(!result.is_error);
        assert_eq!(messages[2].meta_kind.as_deref(), Some("meta"));
        assert_eq!(
            messages[2].blocks[0].text.as_deref(),
            Some("temporary failure")
        );
        assert!(messages.iter().all(|message| {
            message.blocks.iter().all(|block| {
                !block
                    .text
                    .as_deref()
                    .is_some_and(|text| text.contains("Unsupported Grok Build update"))
            })
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_and_last_prompt_use_real_user_text_blocks() {
        let root = scratch("search-prompt");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[
                event(
                    "u1",
                    "p1",
                    "user_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"First prompt"}}),
                ),
                event(
                    "a1",
                    "p1",
                    "agent_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"assistant-only words"}}),
                ),
                event(
                    "u2",
                    "p2",
                    "user_message_chunk",
                    serde_json::json!({"content":{"type":"text","text":"Second searchable prompt"}}),
                ),
            ],
        );
        let source = GrokSource;
        let path = updates.to_string_lossy();
        assert!(source.contains_text(&path, "searchable"));
        let hit = super::super::find_text_hit(
            |candidate| read_updates(Path::new(candidate)),
            &path,
            source_mtime(&updates),
            "searchable",
        )
        .unwrap();
        assert_eq!(hit.msg_index, 2);
        assert!(hit.snippet.contains("searchable"));
        assert_eq!(
            source.last_prompt(&path).unwrap().as_deref(),
            Some("Second searchable prompt")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_commands_use_grok_resume_and_new_session_contracts() {
        let source = GrokSource;
        let resume = source.resume_command("session-123", "ignored");
        assert_eq!(resume.args(), &["--resume", "session-123"]);
        let new_session = source.new_session_command();
        assert!(new_session.args().is_empty());
    }

    #[test]
    fn usage_uses_latest_cumulative_snapshot_without_double_counting_cache_or_reasoning() {
        let root = scratch("usage");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[
                event(
                    "done1",
                    "p1",
                    "turn_completed",
                    serde_json::json!({"usage":{"inputTokens":100,"outputTokens":20,"cachedReadTokens":30,"reasoningTokens":5,"totalTokens":120,"modelCalls":1}}),
                ),
                event(
                    "done2",
                    "p2",
                    "turn_completed",
                    serde_json::json!({"usage":{"inputTokens":180,"outputTokens":50,"cachedReadTokens":60,"reasoningTokens":10,"totalTokens":230,"modelCalls":2}}),
                ),
            ],
        );
        let usage = latest_usage(&updates).unwrap();
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.cache_read_input_tokens, 60);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.reasoning_output_tokens, 10);
        assert_eq!(usage.total, 230);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn turns_delta_cumulative_model_calls_between_prompts() {
        let root = scratch("turn-call-delta");
        let first = event(
            "done1",
            "p1",
            "turn_completed",
            serde_json::json!({"usage":{"inputTokens":100,"outputTokens":20,"cachedReadTokens":30,"reasoningTokens":5,"modelCalls":1}}),
        );
        let second = event(
            "done2",
            "p2",
            "turn_completed",
            serde_json::json!({"usage":{"inputTokens":180,"outputTokens":50,"cachedReadTokens":60,"reasoningTokens":10,"modelCalls":2}}),
        );
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"},"current_model_id":"grok-test"}),
            &[first, second.clone(), second],
        );
        let turns = read_turns_from_updates(&updates).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].calls.len(), 1);
        assert_eq!(turns[1].calls.len(), 1);
        assert_eq!(turns[0].calls[0].call_count, 1);
        assert_eq!(turns[1].calls[0].call_count, 1);
        assert_eq!(turns[0].calls[0].usage.total, 120);
        assert_eq!(turns[1].calls[0].usage.total, 110);
        assert!(
            turns[0].calls[0].pricing_missing || turns[0].calls[0].pricing_estimated,
            "unknown third-party model must be marked missing or estimated"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_namespaced_xai_session_update_method() {
        let root = scratch("namespaced-method");
        let mut user = event(
            "user",
            "p1",
            "user_message_chunk",
            serde_json::json!({"content":{"type":"text","text":"hello"}}),
        );
        let mut assistant = event(
            "assistant",
            "p1",
            "agent_message_chunk",
            serde_json::json!({"content":{"type":"text","text":"world"}}),
        );
        let mut completed = event(
            "done",
            "p1",
            "turn_completed",
            serde_json::json!({"usage":{"inputTokens":100,"outputTokens":20,"modelCalls":2}}),
        );
        for item in [&mut user, &mut assistant, &mut completed] {
            item["method"] = Value::String("_x.ai/session/update".to_string());
        }
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"},"current_model_id":"grok-namespaced-test"}),
            &[user, assistant, completed],
        );

        let messages = read_updates(&updates).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].blocks[0].text.as_deref(), Some("hello"));
        assert_eq!(messages[1].blocks[0].text.as_deref(), Some("world"));

        let turns = read_turns_from_updates(&updates).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].calls.len(), 1);
        assert_eq!(turns[0].calls[0].call_count, 2);
        assert_eq!(turns[0].calls[0].usage.total, 120);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tool_only_turn_keeps_tool_analytics_without_inventing_model_call() {
        let root = scratch("tool-only-zero-weight");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[event(
                "tool",
                "p1",
                "tool_call",
                serde_json::json!({"title":"read_file","rawInput":{"path":"safe.txt"}}),
            )],
        );

        let turns = read_turns_from_updates(&updates).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].calls.len(), 1);
        assert_eq!(turns[0].calls[0].call_count, 0);
        assert!(turns[0].calls[0].model.is_empty());
        assert_eq!(turns[0].calls[0].tools, vec!["read_file"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn turns_use_model_level_usage_and_keep_model_calls_as_weight() {
        let root = scratch("model-usage-weight");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[
                event(
                    "done1",
                    "p1",
                    "turn_completed",
                    serde_json::json!({"usage":{
                        "inputTokens":9999,
                        "outputTokens":9999,
                        "modelCalls":5,
                        "modelUsage":{
                            "grok-model-a-test":{"inputTokens":100,"outputTokens":20,"cachedReadTokens":30,"reasoningTokens":5,"modelCalls":2},
                            "grok-model-b-test":{"inputTokens":200,"outputTokens":50,"cachedReadTokens":40,"reasoningTokens":10,"modelCalls":3}
                        }
                    }}),
                ),
                event(
                    "done2",
                    "p2",
                    "turn_completed",
                    serde_json::json!({"usage":{
                        "inputTokens":19999,
                        "outputTokens":19999,
                        "modelCalls":8,
                        "modelUsage":{
                            "grok-model-a-test":{"inputTokens":160,"outputTokens":35,"cachedReadTokens":50,"reasoningTokens":8,"modelCalls":3},
                            "grok-model-b-test":{"inputTokens":260,"outputTokens":65,"cachedReadTokens":50,"reasoningTokens":12,"modelCalls":5}
                        }
                    }}),
                ),
            ],
        );

        let turns = read_turns_from_updates(&updates).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].calls.len(), 2, "one aggregate record per model");
        assert_eq!(turns[1].calls.len(), 2, "one aggregate record per model");

        let first_calls: u64 = turns[0].calls.iter().map(|call| call.call_count).sum();
        let second_calls: u64 = turns[1].calls.iter().map(|call| call.call_count).sum();
        assert_eq!(first_calls, 5);
        assert_eq!(second_calls, 3);

        let first_usage = turns[0].total_usage();
        let second_usage = turns[1].total_usage();
        assert_eq!(first_usage.input_tokens, 230);
        assert_eq!(first_usage.cache_read_input_tokens, 70);
        assert_eq!(first_usage.output_tokens, 55);
        assert_eq!(first_usage.reasoning_output_tokens, 15);
        assert_eq!(first_usage.total, 370);
        assert_eq!(second_usage.input_tokens, 90);
        assert_eq!(second_usage.cache_read_input_tokens, 30);
        assert_eq!(second_usage.output_tokens, 25);
        assert_eq!(second_usage.reasoning_output_tokens, 5);
        assert_eq!(second_usage.total, 150);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn context_watch_target_and_mtime_include_grok_sidecar_state() {
        let root = scratch("watch-context");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[],
        );
        let session = updates.parent().unwrap();
        let signals = session.join("signals.json");
        fs::write(&signals, r#"{"contextTokensUsed":321}"#).unwrap();

        let source = GrokSource;
        let context = source.context_usage(&updates.to_string_lossy()).unwrap();
        assert_eq!(context.input_tokens, 321);
        assert_eq!(
            source.watch_target(&updates.to_string_lossy()),
            Some(updates.clone())
        );
        assert_eq!(
            source_mtime(&updates),
            [
                mtime_millis(&updates),
                mtime_millis(&session.join(SUMMARY_FILE)),
                mtime_millis(&signals),
            ]
            .into_iter()
            .max()
            .unwrap()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validates_directory_storage_and_rejects_paths_outside_grok_home() {
        let root = scratch("storage");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[],
        );
        let unit = validate_existing_storage(&root, &updates).unwrap();
        assert_eq!(unit.kind, SessionStorageKind::Directory);
        assert_eq!(unit.entry_relative_path, PathBuf::from(UPDATES_FILE));

        let outside = root.join("outside").join(UPDATES_FILE);
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, "").unwrap();
        assert!(validate_existing_storage(&root, &outside).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_session_directories() {
        use std::os::unix::fs::symlink;

        let root = scratch("symlink-storage");
        let real = create_session(
            &root,
            "%2Ftmp%2Freal",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/real"}}),
            &[],
        );
        let linked_group = root.join("sessions").join("%2Ftmp%2Flinked");
        fs::create_dir_all(&linked_group).unwrap();
        let linked_session = linked_group.join("session-link");
        symlink(real.parent().unwrap(), &linked_session).unwrap();

        assert!(validate_existing_storage(&root, &linked_session.join(UPDATES_FILE)).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rename_preserves_unknown_summary_fields() {
        let root = scratch("rename");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({
                "info":{"id":"session-a","cwd":"/tmp/demo"},
                "generated_title":"old",
                "unknown_future_field":{"keep":true}
            }),
            &[],
        );
        fs::write(root.join("session_search.sqlite"), "not a sqlite database").unwrap();
        rename_session_at(&root, &updates, "new").unwrap();
        let saved = read_summary(updates.parent().unwrap());
        assert_eq!(saved["generated_title"], "new");
        assert_eq!(saved["title_is_manual"], true);
        assert_eq!(saved["unknown_future_field"]["keep"], true);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn summary_lock_rejects_a_concurrent_writer() {
        let root = scratch("rename-lock");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[],
        );
        let session = updates.parent().unwrap();
        let lock = acquire_summary_lock(session).unwrap();
        let error = rename_session_at(&root, &updates, "blocked rename").unwrap_err();
        assert!(error.contains("currently locked"));
        drop(lock);
        rename_session_at(&root, &updates, "successful rename").unwrap();
        assert_eq!(
            read_summary(session)["generated_title"],
            "successful rename"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_empty_summary_lock_is_reclaimable_but_viewer_lock_is_not() {
        let root = scratch("stale-rename-lock");
        let updates = create_session(
            &root,
            "%2Ftmp%2Fdemo",
            "session-a",
            serde_json::json!({"info":{"id":"session-a","cwd":"/tmp/demo"}}),
            &[],
        );
        let session = updates.parent().unwrap();
        let lock_path = session.join("summary.json.lock");
        fs::write(&lock_path, b"").unwrap();
        let stale_metadata = fs::metadata(&lock_path).unwrap();
        let old = std::time::SystemTime::now() + SUMMARY_LOCK_STALE_AFTER;
        assert!(is_stale_empty_summary_lock(&stale_metadata, old));
        fs::write(&lock_path, b"viewer-pid=123\n").unwrap();
        let viewer_metadata = fs::metadata(&lock_path).unwrap();
        assert!(!is_stale_empty_summary_lock(&viewer_metadata, old));
        let _ = fs::remove_dir_all(root);
    }
}
