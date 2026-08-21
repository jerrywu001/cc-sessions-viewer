//! Kimi Code local-session source.
//!
//! Kimi stores one user-visible session in a directory. `state.json` is the
//! session metadata and `agents/main/wire.jsonl` is the primary transcript.
//! The parser deliberately remains minimal in this first integration phase;
//! tool/content reconstruction and usage accounting are added separately.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{TimeZone, Utc};
use rayon::prelude::*;
use serde_json::Value;

use super::{SessionSource, SessionStorageKind, SessionStorageUnit};
use crate::agent_command::AgentCommand;
use crate::stats::pricing;
use crate::stats::shell::{extract_first_command, extract_mcp_server};
use crate::stats::types::{CallRecord, Turn};
use crate::types::{
    Block, DiffHunk, DiffLine, Msg, ProjectInfo, SessionMeta, SessionPage, UsageSummary,
};
use crate::util::{
    clean_title, home, mtime_millis, now_millis, text_block, truncate_subtitle,
    validate_rename_name,
};

pub struct KimiSource;

const SESSIONS_DIR: &str = "sessions";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const STATE_FILE: &str = "state.json";
const MAIN_WIRE_RELATIVE: &str = "agents/main/wire.jsonl";
const MAX_TOOL_INPUT_BYTES: usize = 64 * 1024;
const MAX_TOOL_RESULT_BYTES: usize = 128 * 1024;
const MAX_QUESTION_TEXT_BYTES: usize = 8 * 1024;
const MAX_OPTION_LABEL_BYTES: usize = 1024;
const MAX_OPTION_DESCRIPTION_BYTES: usize = 8 * 1024;
const SNAPSHOT_RETRIES: usize = 4;
const SNAPSHOT_RETRY_DELAY: Duration = Duration::from_millis(20);

#[derive(Clone)]
struct KimiSessionRecord {
    session_dir: PathBuf,
    main_wire_path: PathBuf,
    id: String,
    cwd: String,
    title: String,
    created: Option<String>,
    modified: u64,
}

type WireSnapshot = Vec<(PathBuf, Vec<u8>)>;

pub fn kimi_home() -> PathBuf {
    let configured = std::env::var_os("KIMI_CODE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_kimi_home(configured, &home(), &current_dir)
}

pub fn config_path() -> PathBuf {
    kimi_home().join("config.toml")
}

fn resolve_kimi_home(
    configured: Option<PathBuf>,
    default_home: &Path,
    current_dir: &Path,
) -> PathBuf {
    let configured = configured.unwrap_or_else(|| default_home.join(".kimi-code"));
    if configured.is_absolute() {
        configured
    } else {
        current_dir.join(configured)
    }
}

fn sessions_dir(root: &Path) -> PathBuf {
    root.join(SESSIONS_DIR)
}

fn session_index_path(root: &Path) -> PathBuf {
    root.join(SESSION_INDEX_FILE)
}

fn main_wire_path(session_dir: &Path) -> PathBuf {
    session_dir.join(MAIN_WIRE_RELATIVE)
}

fn nonempty_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn read_state(session_dir: &Path) -> Option<Value> {
    fs::read_to_string(session_dir.join(STATE_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .filter(Value::is_object)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileRevision {
    size: u64,
    modified: SystemTime,
    identity: (u64, u64),
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;

    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> (u64, u64) {
    (0, 0)
}

fn file_revision(path: &Path) -> Result<FileRevision, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "Failed to inspect Kimi Code file {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "Kimi Code file is not a regular file: {}",
            path.display()
        ));
    }
    let modified = metadata.modified().map_err(|error| {
        format!(
            "Failed to read Kimi Code file mtime {}: {error}",
            path.display()
        )
    })?;
    Ok(FileRevision {
        size: metadata.len(),
        modified,
        identity: file_identity(&metadata),
    })
}

/// Read a group of Kimi files only when every file remains the same before and
/// after the read. This prevents a partial state.json or wire line from being
/// paired with data from a different revision during live tail/stat scans.
fn read_stable_files(paths: &[PathBuf], state_index: usize) -> Result<Vec<Vec<u8>>, String> {
    for attempt in 0..SNAPSHOT_RETRIES {
        let before = paths
            .iter()
            .map(|path| file_revision(path))
            .collect::<Result<Vec<_>, _>>()?;
        let bytes = paths
            .iter()
            .map(|path| {
                fs::read(path).map_err(|error| {
                    format!("Failed to read Kimi Code file {}: {error}", path.display())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let valid_state = serde_json::from_slice::<Value>(&bytes[state_index])
            .ok()
            .is_some_and(|state| state.is_object());
        let after = paths
            .iter()
            .map(|path| file_revision(path))
            .collect::<Result<Vec<_>, _>>()?;
        if valid_state && before == after {
            return Ok(bytes);
        }
        if attempt + 1 < SNAPSHOT_RETRIES {
            std::thread::sleep(SNAPSHOT_RETRY_DELAY);
        }
    }
    Err("Kimi Code session changed while reading; retry later".to_string())
}

fn session_dir_for_main_wire(path: &Path) -> Result<PathBuf, String> {
    path.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "Invalid Kimi Code main wire path".to_string())
}

fn read_stable_main_wire(path: &Path) -> Result<Vec<u8>, String> {
    let session_dir = session_dir_for_main_wire(path)?;
    let paths = vec![session_dir.join(STATE_FILE), path.to_path_buf()];
    Ok(read_stable_files(&paths, 0)?[1].clone())
}

fn state_timestamp(state: &Value, key: &str) -> Option<String> {
    state.get(key).and_then(|value| match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn parse_timestamp_millis(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc3339(value)
            .ok()
            .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
    })
}

fn state_modified(state: &Value, session_dir: &Path) -> u64 {
    ["updatedAt", "createdAt"]
        .into_iter()
        .filter_map(|key| {
            state_timestamp(state, key).and_then(|value| parse_timestamp_millis(&value))
        })
        .chain(std::iter::once(session_files_mtime(session_dir)))
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
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| directory_size(&entry.path()))
        .sum()
}

fn session_files_mtime(session_dir: &Path) -> u64 {
    let state_mtime = mtime_millis(&session_dir.join(STATE_FILE));
    let agents_dir = session_dir.join("agents");
    let mut maximum = state_mtime;
    let Ok(agent_dirs) = fs::read_dir(agents_dir) else {
        return maximum;
    };
    for agent_dir in agent_dirs.flatten() {
        let Ok(file_type) = agent_dir.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        maximum = maximum.max(mtime_millis(&agent_dir.path().join("wire.jsonl")));
    }
    maximum
}

fn is_regular_non_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn canonical_sessions_root(root: &Path) -> Result<PathBuf, String> {
    sessions_dir(root)
        .canonicalize()
        .map_err(|error| format!("Failed to resolve Kimi Code sessions directory: {error}"))
}

fn valid_session_dir(root: &Path, session_dir: &Path) -> Option<PathBuf> {
    let sessions_root = canonical_sessions_root(root).ok()?;
    let group_dir = session_dir.parent()?;
    for path in [group_dir, session_dir] {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return None;
        }
    }
    if !session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("session_"))
    {
        return None;
    }
    let canonical_session = session_dir.canonicalize().ok()?;
    if canonical_session.parent()?.parent()? != sessions_root {
        return None;
    }
    let state_path = canonical_session.join(STATE_FILE);
    let wire_path = main_wire_path(&canonical_session);
    if !is_regular_non_symlink(&state_path) || !is_regular_non_symlink(&wire_path) {
        return None;
    }
    Some(canonical_session)
}

fn index_session_dirs(root: &Path) -> Vec<PathBuf> {
    let index = session_index_path(root);
    let Ok(file) = fs::File::open(index) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter_map(|entry| {
            let path = entry
                .get("sessionDir")
                .or_else(|| entry.get("session_dir"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let path = PathBuf::from(path);
            Some(if path.is_absolute() {
                path
            } else {
                sessions_dir(root).join(path)
            })
        })
        .collect()
}

fn scanned_session_dirs(root: &Path) -> Vec<PathBuf> {
    let base = sessions_dir(root);
    let Ok(groups) = fs::read_dir(base) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for group in groups.flatten() {
        let Ok(group_type) = group.file_type() else {
            continue;
        };
        if !group_type.is_dir() || group_type.is_symlink() {
            continue;
        }
        let Ok(entries) = fs::read_dir(group.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(entry_type) = entry.file_type() else {
                continue;
            };
            if entry_type.is_dir() && !entry_type.is_symlink() {
                sessions.push(entry.path());
            }
        }
    }
    sessions
}

fn prompt_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.trim().to_string(),
        Value::Array(values) => values
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .or_else(|| item.get("text").and_then(Value::as_str))
                    .or_else(|| item.get("content").and_then(Value::as_str))
            })
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => value
            .get("text")
            .or_else(|| value.get("content"))
            .map(prompt_text)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn kimi_prompt_image_src(value: &Value) -> Option<String> {
    let url = value
        .pointer("/imageUrl/url")
        .or_else(|| value.pointer("/image_url/url"))
        .or_else(|| value.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())?;
    matches!(url, url if url.starts_with("data:image/") || url.starts_with("http:") || url.starts_with("https:"))
        .then(|| url.to_string())
}

/// Kimi's Windows prompt protocol interleaves `{type:"text"}` and
/// `{type:"image_url", imageUrl:{url:"data:image/..."}}` items. Keep that
/// order for the transcript; the text-only helper above remains responsible
/// for list titles and search indexing.
fn prompt_blocks(value: &Value) -> Vec<Block> {
    let Some(items) = value.as_array() else {
        let text = prompt_text(value);
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![text_block("text", &text)]
        };
    };

    let has_image_token = items.iter().any(|item| {
        item.get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("[Image #"))
    });
    let image_count = items
        .iter()
        .filter(|item| {
            matches!(
                item.get("type").and_then(Value::as_str),
                Some("image_url" | "imageUrl" | "image")
            ) && kimi_prompt_image_src(item).is_some()
        })
        .count();
    let mut image_index = 0usize;
    let mut blocks = Vec::with_capacity(items.len());
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("image_url" | "imageUrl" | "image") => {
                let Some(src) = kimi_prompt_image_src(item) else {
                    continue;
                };
                image_index += 1;
                blocks.push(Block {
                    kind: "image".to_string(),
                    image_src: Some(src),
                    // The Windows wire has no visible image token. This is an
                    // input image, not an ordinary attachment, so retain the
                    // UI's pasted-image tag without adding text that Kimi did
                    // not record. If Kimi did write `[Image #N]`, defer to the
                    // shared binder to preserve its exact numbering.
                    inline_placeholder: (!has_image_token && image_count > 0)
                        .then(|| format!("[Image #{image_index}]")),
                    ..Default::default()
                });
            }
            _ => {
                let text = item
                    .get("text")
                    .or_else(|| item.get("content"))
                    .map(prompt_text)
                    .unwrap_or_default();
                if !text.is_empty() {
                    blocks.push(text_block("text", &text));
                }
            }
        }
    }
    blocks
}

fn is_user_prompt(event: &Value) -> bool {
    event
        .pointer("/origin/kind")
        .and_then(Value::as_str)
        .map(|kind| kind == "user")
        // Older records did not always persist origin metadata. Treat a
        // missing origin as a legacy user prompt, but never accept a known
        // system-triggered prompt as user-visible conversation text.
        .unwrap_or(true)
}

fn main_user_prompts(path: &Path) -> Result<Vec<(Option<String>, String)>, String> {
    let bytes = read_stable_main_wire(path)?;
    main_user_prompts_from_bytes(&bytes)
}

fn main_user_prompts_from_bytes(bytes: &[u8]) -> Result<Vec<(Option<String>, String)>, String> {
    let mut prompts = Vec::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("turn.prompt")
            || !is_user_prompt(&event)
        {
            continue;
        }
        let text = event.get("input").map(prompt_text).unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        let uuid = event
            .get("promptId")
            .or_else(|| event.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        prompts.push((uuid, text));
    }
    Ok(prompts)
}

fn timestamp_from_millis(value: &Value) -> Option<String> {
    let millis = value.as_i64()?;
    Utc.timestamp_millis_opt(millis)
        .single()
        .map(|timestamp| timestamp.to_rfc3339())
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &value[..end])
}

fn bounded_json(value: &Value, limit: usize) -> String {
    bounded_text(
        &serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
        limit,
    )
}

fn required_short_string(value: &Value, key: &str, limit: usize) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty() && text.len() <= limit)
        .map(str::to_string)
}

fn normalize_ask_user_question_args(args: &Value) -> Option<Value> {
    let questions = args.get("questions")?.as_array()?;
    if !(1..=4).contains(&questions.len())
        || bounded_json(args, MAX_TOOL_INPUT_BYTES).len() > MAX_TOOL_INPUT_BYTES
    {
        return None;
    }
    let mut seen_questions = HashSet::new();
    let mut normalized = Vec::with_capacity(questions.len());
    for question in questions {
        let question_text = required_short_string(question, "question", MAX_QUESTION_TEXT_BYTES)?;
        if !seen_questions.insert(question_text.clone()) {
            return None;
        }
        let options = question.get("options")?.as_array()?;
        if !(2..=4).contains(&options.len()) {
            return None;
        }
        let mut seen_options = HashSet::new();
        let mut normalized_options = Vec::with_capacity(options.len());
        for option in options {
            let label = required_short_string(option, "label", MAX_OPTION_LABEL_BYTES)?;
            if !seen_options.insert(label.clone()) {
                return None;
            }
            let mut normalized_option = serde_json::Map::new();
            normalized_option.insert("label".to_string(), Value::String(label));
            if let Some(description) =
                required_short_string(option, "description", MAX_OPTION_DESCRIPTION_BYTES)
            {
                normalized_option.insert("description".to_string(), Value::String(description));
            }
            if let Some(preview) =
                required_short_string(option, "preview", MAX_OPTION_DESCRIPTION_BYTES)
            {
                normalized_option.insert("preview".to_string(), Value::String(preview));
            }
            normalized_options.push(Value::Object(normalized_option));
        }
        let mut normalized_question = serde_json::Map::new();
        normalized_question.insert("question".to_string(), Value::String(question_text));
        normalized_question.insert("options".to_string(), Value::Array(normalized_options));
        if let Some(header) = required_short_string(question, "header", MAX_OPTION_LABEL_BYTES) {
            normalized_question.insert("header".to_string(), Value::String(header));
        }
        if question
            .get("multiSelect")
            .or_else(|| question.get("multi_select"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            normalized_question.insert("multiSelect".to_string(), Value::Bool(true));
        }
        if question.get("allowOther").and_then(Value::as_bool) == Some(false) {
            normalized_question.insert("allowOther".to_string(), Value::Bool(false));
        }
        normalized.push(Value::Object(normalized_question));
    }
    let mut result = serde_json::Map::new();
    result.insert("questions".to_string(), Value::Array(normalized));
    if args.get("background").and_then(Value::as_bool) == Some(true) {
        result.insert("background".to_string(), Value::Bool(true));
    }
    Some(Value::Object(result))
}

fn loop_key(event: &Value) -> String {
    let turn = event.get("turnId").and_then(Value::as_str).unwrap_or("?");
    let step = event
        .get("stepUuid")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            event
                .get("step")
                .and_then(Value::as_i64)
                .map(|step| step.to_string())
        })
        .unwrap_or_else(|| "?".to_string());
    format!("{turn}:{step}")
}

fn assistant_message(timestamp: Option<String>) -> Msg {
    Msg {
        uuid: None,
        role: "assistant".to_string(),
        timestamp,
        model: None,
        sidechain: false,
        blocks: Vec::new(),
        meta_kind: None,
    }
}

fn tool_result_text(result: &Value) -> String {
    match result {
        Value::Object(result) => {
            let mut parts = Vec::new();
            if let Some(output) = result.get("output").and_then(Value::as_str) {
                parts.push(output.to_string());
            }
            if let Some(note) = result.get("note").and_then(Value::as_str) {
                parts.push(note.to_string());
            }
            if parts.is_empty() {
                bounded_text(
                    &serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string()),
                    MAX_TOOL_RESULT_BYTES,
                )
            } else {
                bounded_text(&parts.join("\n"), MAX_TOOL_RESULT_BYTES)
            }
        }
        Value::String(result) => bounded_text(result, MAX_TOOL_RESULT_BYTES),
        _ => bounded_json(result, MAX_TOOL_RESULT_BYTES),
    }
}

/// Normalize Kimi's file mutation arguments into the shared file-change shape
/// used by the Codex/Claude renderers. Kimi sends Edit/Write as tool calls with
/// `path` plus old/new content rather than a separate structured diff result.
fn kimi_file_change(
    args: &Value,
    name: &str,
) -> (Option<String>, Option<String>, Option<Vec<DiffHunk>>) {
    let path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|path| !path.is_empty());
    let Some(path) = path else {
        return (None, None, None);
    };
    let lower = name.to_ascii_lowercase();
    let change_type = if lower == "write" {
        "add"
    } else if lower == "delete" {
        "delete"
    } else if matches!(lower.as_str(), "edit" | "multiedit" | "notebookedit") {
        "update"
    } else {
        return (None, None, None);
    };

    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| args.get(*key).and_then(Value::as_str))
    };
    let lines = match change_type {
        "add" => text(&["content", "new_string", "newString"]).map(|content| {
            content
                .split('\n')
                .enumerate()
                .map(|(index, line)| DiffLine {
                    kind: "add".to_string(),
                    old_no: None,
                    new_no: Some(index as u32 + 1),
                    text: line.to_string(),
                })
                .collect::<Vec<_>>()
        }),
        "update" => {
            let old = text(&["old_string", "oldString"]).unwrap_or_default();
            let new = text(&["new_string", "newString"]).unwrap_or_default();
            let mut lines = Vec::new();
            lines.extend(old.split('\n').enumerate().map(|(index, line)| DiffLine {
                kind: "del".to_string(),
                old_no: Some(index as u32 + 1),
                new_no: None,
                text: line.to_string(),
            }));
            lines.extend(new.split('\n').enumerate().map(|(index, line)| DiffLine {
                kind: "add".to_string(),
                old_no: None,
                new_no: Some(index as u32 + 1),
                text: line.to_string(),
            }));
            Some(lines)
        }
        "delete" => None,
        _ => None,
    };
    let diff = lines.map(|lines| {
        vec![DiffHunk {
            old_start: if change_type == "add" { 0 } else { 1 },
            new_start: if change_type == "delete" { 0 } else { 1 },
            lines,
        }]
    });
    (Some(path), Some(change_type.to_string()), diff)
}

fn fallback_append_messages(events: &[Value]) -> Vec<Msg> {
    let mut messages = Vec::new();
    for event in events {
        if event.get("type").and_then(Value::as_str) != Some("context.append_message") {
            continue;
        }
        let Some(message) = event.get("message") else {
            continue;
        };
        let role = message.get("role").and_then(Value::as_str);
        if !matches!(role, Some("user") | Some("assistant")) {
            continue;
        }
        if role == Some("user")
            && message
                .pointer("/origin/kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != "user")
        {
            continue;
        }
        let text = message.get("content").map(prompt_text).unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        messages.push(Msg {
            uuid: message
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string),
            role: role.unwrap_or_default().to_string(),
            timestamp: event.get("time").and_then(timestamp_from_millis),
            model: None,
            sidechain: false,
            blocks: vec![text_block("text", &text)],
            meta_kind: None,
        });
    }
    messages
}

#[cfg(test)]
fn read_main_wire(path: &Path) -> Result<Vec<Msg>, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("Failed to open Kimi Code session: {error}"))?;
    parse_main_wire_bytes(&bytes)
}

fn parse_main_wire_bytes(bytes: &[u8]) -> Result<Vec<Msg>, String> {
    let events: Vec<Value> = String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();
    let has_primary_events = events.iter().any(|event| {
        matches!(
            event.get("type").and_then(Value::as_str),
            Some("turn.prompt") | Some("context.append_loop_event")
        )
    });
    if !has_primary_events {
        return Ok(fallback_append_messages(&events));
    }

    let mut messages = Vec::new();
    let mut assistant_by_step: HashMap<String, usize> = HashMap::new();
    let mut tool_locations: HashMap<String, (usize, usize)> = HashMap::new();
    let mut ambiguous_tool_ids = HashSet::new();
    let mut seen_event_uuids = HashSet::new();

    for event in events {
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if event_type == "turn.prompt" {
            if !is_user_prompt(&event) {
                continue;
            }
            let blocks = event.get("input").map(prompt_blocks).unwrap_or_default();
            if blocks.is_empty() {
                continue;
            }
            messages.push(Msg {
                uuid: event
                    .get("promptId")
                    .or_else(|| event.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                role: "user".to_string(),
                timestamp: event.get("time").and_then(timestamp_from_millis),
                model: None,
                sidechain: false,
                blocks,
                meta_kind: None,
            });
            continue;
        }
        if event_type != "context.append_loop_event" {
            continue;
        }
        let Some(loop_event) = event.get("event") else {
            continue;
        };
        let loop_type = loop_event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let event_uuid = loop_event.get("uuid").and_then(Value::as_str);
        if matches!(loop_type, "content.part" | "tool.call")
            && event_uuid.is_some_and(|uuid| !seen_event_uuids.insert(uuid.to_string()))
        {
            continue;
        }
        let step_key = loop_key(loop_event);
        let timestamp = event.get("time").and_then(timestamp_from_millis);
        let assistant_index =
            |messages: &mut Vec<Msg>, assistant_by_step: &mut HashMap<String, usize>| {
                *assistant_by_step
                    .entry(step_key.clone())
                    .or_insert_with(|| {
                        messages.push(assistant_message(timestamp.clone()));
                        messages.len() - 1
                    })
            };
        match loop_type {
            "content.part" => {
                let Some(part) = loop_event.get("part") else {
                    continue;
                };
                let block = match part.get("type").and_then(Value::as_str) {
                    Some("text") => part
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(|text| text_block("text", text)),
                    Some("think") | Some("thinking") => part
                        .get("think")
                        .or_else(|| part.get("text"))
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(|text| text_block("thinking", text)),
                    _ => None,
                };
                if let Some(block) = block {
                    let index = assistant_index(&mut messages, &mut assistant_by_step);
                    messages[index].blocks.push(block);
                }
            }
            "tool.call" => {
                let Some(name) = loop_event.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let tool_call_id = loop_event.get("toolCallId").and_then(Value::as_str);
                let input = if name == "AskUserQuestion" {
                    normalize_ask_user_question_args(loop_event.get("args").unwrap_or(&Value::Null))
                        .map(|value| bounded_json(&value, MAX_TOOL_INPUT_BYTES))
                        .unwrap_or_else(|| {
                            bounded_json(
                                loop_event.get("args").unwrap_or(&Value::Null),
                                MAX_TOOL_INPUT_BYTES,
                            )
                        })
                } else {
                    bounded_json(
                        loop_event.get("args").unwrap_or(&Value::Null),
                        MAX_TOOL_INPUT_BYTES,
                    )
                };
                let index = assistant_index(&mut messages, &mut assistant_by_step);
                let block_index = messages[index].blocks.len();
                let mut block = Block {
                    kind: "tool_use".to_string(),
                    tool_name: Some(name.to_string()),
                    tool_input: Some(input),
                    tool_id: tool_call_id.map(str::to_string),
                    ..Default::default()
                };
                let (file_path, file_change_type, diff) =
                    kimi_file_change(loop_event.get("args").unwrap_or(&Value::Null), name);
                block.file_path = file_path;
                block.file_change_type = file_change_type;
                block.diff = diff;
                if let Some(tool_call_id) = tool_call_id {
                    if let Some((previous_message, previous_block)) =
                        tool_locations.get(tool_call_id).copied()
                    {
                        messages[previous_message].blocks[previous_block].tool_id = None;
                        ambiguous_tool_ids.insert(tool_call_id.to_string());
                        block.tool_id = None;
                    } else if !ambiguous_tool_ids.contains(tool_call_id) {
                        tool_locations.insert(tool_call_id.to_string(), (index, block_index));
                    }
                }
                messages[index].blocks.push(block);
            }
            "tool.result" => {
                let result = loop_event.get("result").unwrap_or(&Value::Null);
                let tool_id = loop_event.get("toolCallId").and_then(Value::as_str);
                let is_error = result
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                messages.push(Msg {
                    uuid: event_uuid.map(str::to_string),
                    role: "user".to_string(),
                    timestamp,
                    model: None,
                    sidechain: false,
                    blocks: vec![Block {
                        kind: "tool_result".to_string(),
                        text: Some(tool_result_text(result)),
                        tool_id: tool_id.map(str::to_string),
                        is_error,
                        ..Default::default()
                    }],
                    meta_kind: None,
                });
            }
            _ => {}
        }
    }
    Ok(messages)
}

fn record_from_session_dir(root: &Path, session_dir: &Path) -> Option<KimiSessionRecord> {
    let session_dir = valid_session_dir(root, session_dir)?;
    let state = read_state(&session_dir)?;
    let cwd = nonempty_string(&state, "cwd")?.to_string();
    let directory_id = session_dir.file_name()?.to_string_lossy().to_string();
    let id = nonempty_string(&state, "id")
        .map(str::to_owned)
        .filter(|id| !id.is_empty())
        .unwrap_or(directory_id);
    let main_wire_path = main_wire_path(&session_dir);
    let fallback_prompt = nonempty_string(&state, "lastPrompt")
        .map(clean_title)
        .filter(|title| !title.is_empty())
        .or_else(|| {
            main_user_prompts(&main_wire_path)
                .ok()
                .and_then(|prompts| {
                    prompts
                        .into_iter()
                        .next()
                        .map(|(_, text)| clean_title(&text))
                })
                .filter(|title| !title.is_empty())
        });
    let title = nonempty_string(&state, "title")
        .map(clean_title)
        .filter(|title| !title.is_empty())
        .or(fallback_prompt)
        .unwrap_or_else(|| id.clone());
    Some(KimiSessionRecord {
        created: state_timestamp(&state, "createdAt"),
        modified: state_modified(&state, &session_dir),
        session_dir,
        main_wire_path,
        id,
        cwd,
        title,
    })
}

fn discover_session_records(root: &Path) -> Result<Vec<KimiSessionRecord>, String> {
    let base = sessions_dir(root);
    if !base.exists() {
        return Ok(Vec::new());
    }
    let _ = fs::read_dir(&base)
        .map_err(|error| format!("Failed to read Kimi Code sessions directory: {error}"))?;
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    for candidate in index_session_dirs(root)
        .into_iter()
        .chain(scanned_session_dirs(root))
    {
        let Some(session_dir) = valid_session_dir(root, &candidate) else {
            continue;
        };
        if !seen.insert(session_dir.clone()) {
            continue;
        }
        if let Some(record) = record_from_session_dir(root, &session_dir) {
            records.push(record);
        }
    }
    Ok(records)
}

/// Resolve a Kimi hook's session identifier to the real primary wire path.
/// Hooks only provide `session_id` and `cwd`; the existing discovery validation
/// keeps this lookup inside KIMI_CODE_HOME/sessions.
pub fn find_main_wire_path(session_id: &str, cwd: Option<&str>) -> Option<PathBuf> {
    find_main_wire_path_at(&kimi_home(), session_id, cwd)
}

fn find_main_wire_path_at(root: &Path, session_id: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let cwd = cwd.map(str::trim).filter(|cwd| !cwd.is_empty());
    discover_session_records(root)
        .ok()?
        .into_iter()
        .find(|record| record.id == session_id && cwd.is_none_or(|cwd| record.cwd == cwd))
        .map(|record| record.main_wire_path)
}

fn session_meta(record: &KimiSessionRecord) -> SessionMeta {
    let message_count = main_user_prompts(&record.main_wire_path)
        .map(|prompts| prompts.len())
        .unwrap_or(0);
    SessionMeta {
        id: record.id.clone(),
        file_name: "wire.jsonl".to_string(),
        path: record.main_wire_path.to_string_lossy().to_string(),
        title: record.title.clone(),
        cwd: Some(record.cwd.clone()),
        created: record.created.clone(),
        modified: record.modified,
        size: directory_size(&record.session_dir),
        message_count,
        codex_app_list_rank: None,
        codex_app_list_scanned: 0,
        codex_app_first_page_size: 0,
        codex_app_first_page_position: 0,
        codex_internal: false,
        codex_archived: false,
    }
}

fn validate_existing_storage(root: &Path, main_wire: &Path) -> Result<SessionStorageUnit, String> {
    if main_wire.file_name().and_then(|name| name.to_str()) != Some("wire.jsonl") {
        return Err("Kimi Code session path must point to agents/main/wire.jsonl".to_string());
    }
    let main_dir = main_wire
        .parent()
        .ok_or_else(|| "Invalid Kimi Code main agent directory".to_string())?;
    if main_dir.file_name().and_then(|name| name.to_str()) != Some("main")
        || main_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("agents")
    {
        return Err("Kimi Code session path must point to agents/main/wire.jsonl".to_string());
    }
    if !is_regular_non_symlink(main_wire) {
        return Err("Kimi Code main wire.jsonl does not exist or is a symlink".to_string());
    }
    let session_dir = main_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "Invalid Kimi Code session directory".to_string())?;
    let canonical_session = valid_session_dir(root, session_dir)
        .ok_or_else(|| "Kimi Code session path is outside KIMI_CODE_HOME/sessions".to_string())?;
    let canonical_wire = main_wire
        .canonicalize()
        .map_err(|error| format!("Failed to resolve Kimi Code main wire path: {error}"))?;
    if canonical_wire != main_wire_path(&canonical_session) {
        return Err("Kimi Code session path is outside KIMI_CODE_HOME/sessions".to_string());
    }
    Ok(SessionStorageUnit {
        root_path: canonical_session,
        entry_relative_path: PathBuf::from(MAIN_WIRE_RELATIVE),
        kind: SessionStorageKind::Directory,
    })
}

fn validate_restore_storage(
    root: &Path,
    entry_path: &Path,
    session_dir: &Path,
) -> Result<(), String> {
    if !entry_path.is_absolute() || !session_dir.is_absolute() {
        return Err("Kimi Code restore target must be an absolute path".to_string());
    }
    if entry_path != main_wire_path(session_dir) {
        return Err("Kimi Code restore entry must be agents/main/wire.jsonl".to_string());
    }
    let configured_sessions_root = sessions_dir(root);
    let sessions_root = configured_sessions_root
        .canonicalize()
        .unwrap_or_else(|_| configured_sessions_root.clone());
    let relative = session_dir
        .strip_prefix(&configured_sessions_root)
        .or_else(|_| session_dir.strip_prefix(&sessions_root))
        .map_err(|_| "Kimi Code restore target is outside KIMI_CODE_HOME/sessions".to_string())?;
    let components: Vec<Component<'_>> = relative.components().collect();
    if components.len() != 2
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !components[1]
            .as_os_str()
            .to_string_lossy()
            .starts_with("session_")
    {
        return Err("Invalid Kimi Code restore directory structure".to_string());
    }
    if let Some(group) = session_dir.parent().filter(|group| group.exists()) {
        let metadata = fs::symlink_metadata(group).map_err(|error| {
            format!("Failed to inspect Kimi Code restore project group: {error}")
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Kimi Code restore project group cannot be a symlink".to_string());
        }
        let canonical_group = group.canonicalize().map_err(|error| {
            format!("Failed to resolve Kimi Code restore project group: {error}")
        })?;
        if canonical_group.parent() != Some(sessions_root.as_path()) {
            return Err(
                "Kimi Code restore project group escapes KIMI_CODE_HOME/sessions".to_string(),
            );
        }
    }
    Ok(())
}

fn index_lines(root: &Path) -> Result<Vec<String>, String> {
    let path = session_index_path(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Failed to inspect Kimi Code session index: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Kimi Code session index must be a regular file".to_string());
    }
    fs::read_to_string(path)
        .map(|content| content.lines().map(str::to_string).collect())
        .map_err(|error| format!("Failed to read Kimi Code session index: {error}"))
}

fn index_line_session_id(line: &str) -> Option<String> {
    serde_json::from_str::<Value>(line)
        .ok()?
        .get("sessionId")?
        .as_str()
        .map(str::to_string)
}

fn write_index_lines(root: &Path, lines: &[String]) -> Result<(), String> {
    let path = session_index_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| "Kimi Code session index has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create Kimi Code home directory: {error}"))?;
    if path.exists()
        && fs::symlink_metadata(&path)
            .map(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
            .unwrap_or(true)
    {
        return Err("Kimi Code session index must be a regular file".to_string());
    }
    let temporary = parent.join(format!(
        ".{SESSION_INDEX_FILE}.viewer-{}-{}.tmp",
        std::process::id(),
        now_millis()
    ));
    let mut content = lines.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Failed to create Kimi Code session index temp file: {error}"))?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Failed to write Kimi Code session index: {error}"));
    }
    drop(file);
    if let Err(first_error) = fs::rename(&temporary, &path) {
        // Windows does not replace an existing target with rename. Keep the
        // old index in the same directory until the replacement is installed.
        let backup = parent.join(format!(
            ".{SESSION_INDEX_FILE}.viewer-{}-{}.bak",
            std::process::id(),
            now_millis()
        ));
        if !path.exists() || fs::rename(&path, &backup).is_err() {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "Failed to replace Kimi Code session index: {first_error}"
            ));
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::rename(&backup, &path);
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "Failed to install Kimi Code session index: {error}"
            ));
        }
        let _ = fs::remove_file(backup);
    }
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn state_session_id(session_dir: &Path) -> Result<String, String> {
    let state = read_state(session_dir)
        .ok_or_else(|| "Failed to read Kimi Code session state.json".to_string())?;
    nonempty_string(&state, "id")
        .map(str::to_string)
        .or_else(|| {
            session_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Kimi Code session has no ID".to_string())
}

fn kimi_index_metadata(root: &Path, unit: &SessionStorageUnit) -> Result<Value, String> {
    let session_id = state_session_id(&unit.root_path)?;
    let entries: Vec<Value> = index_lines(root)?
        .into_iter()
        .filter(|line| index_line_session_id(line).as_deref() == Some(session_id.as_str()))
        .map(Value::String)
        .collect();
    Ok(serde_json::json!({
        "sessionId": session_id,
        "indexEntries": entries,
        "sessionRevision": session_revision_stamp(root, &unit.entry_path())?,
    }))
}

fn verify_session_revision(
    root: &Path,
    unit: &SessionStorageUnit,
    metadata: &Value,
) -> Result<(), String> {
    let expected = metadata
        .get("sessionRevision")
        .ok_or_else(|| "Kimi Code trash metadata is missing session revision".to_string())?;
    if session_revision_stamp(root, &unit.entry_path())? != *expected {
        return Err("Kimi Code session changed while preparing delete; try again".to_string());
    }
    Ok(())
}

fn metadata_session_id(metadata: &Value) -> Result<&str, String> {
    metadata
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Kimi Code trash metadata is missing session ID".to_string())
}

fn remove_index_session(root: &Path, session_id: &str) -> Result<(), String> {
    let lines = index_lines(root)?;
    let retained: Vec<String> = lines
        .iter()
        .filter(|line| index_line_session_id(line).as_deref() != Some(session_id))
        .cloned()
        .collect();
    if retained.len() != lines.len() {
        write_index_lines(root, &retained)?;
    }
    Ok(())
}

fn valid_saved_index_entry(line: &str, session_id: &str, session_dir: &Path) -> bool {
    let Ok(entry) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    if entry.get("sessionId").and_then(Value::as_str) != Some(session_id) {
        return false;
    }
    let Some(saved_dir) = entry.get("sessionDir").and_then(Value::as_str) else {
        return false;
    };
    Path::new(saved_dir)
        .canonicalize()
        .map(|path| path == session_dir)
        .unwrap_or(false)
}

fn restore_index_session(
    root: &Path,
    unit: &SessionStorageUnit,
    metadata: &Value,
) -> Result<(), String> {
    let validated = validate_existing_storage(root, &unit.entry_path())?;
    let session_id = state_session_id(&validated.root_path)?;
    if let Some(saved_id) = metadata.get("sessionId").and_then(Value::as_str) {
        if saved_id != session_id {
            return Err(
                "Kimi Code trash metadata session ID does not match state.json".to_string(),
            );
        }
    }
    let state = read_state(&validated.root_path)
        .ok_or_else(|| "Failed to read restored Kimi Code state.json".to_string())?;
    let saved_entry = metadata
        .get("indexEntries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|line| valid_saved_index_entry(line, &session_id, &validated.root_path));
    let entry = saved_entry.map(str::to_string).unwrap_or_else(|| {
        serde_json::json!({
            "sessionId": session_id,
            "sessionDir": validated.root_path,
            "workDir": nonempty_string(&state, "cwd").unwrap_or_default(),
        })
        .to_string()
    });
    let lines = index_lines(root)?;
    let mut retained: Vec<String> = lines
        .into_iter()
        .filter(|line| index_line_session_id(line).as_deref() != Some(session_id.as_str()))
        .collect();
    retained.push(entry);
    write_index_lines(root, &retained)
}

fn write_state_atomically(
    path: &Path,
    state: &Value,
    expected_revision: &FileRevision,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Kimi Code state.json has no parent directory".to_string())?;
    let temporary = parent.join(format!(
        ".{STATE_FILE}.viewer-{}-{}.tmp",
        std::process::id(),
        now_millis()
    ));
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Failed to serialize Kimi Code state.json: {error}"))?;
    bytes.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Failed to create Kimi Code state.json temp file: {error}"))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Failed to write Kimi Code state.json: {error}"));
    }
    drop(file);
    if file_revision(path)? != *expected_revision {
        let _ = fs::remove_file(&temporary);
        return Err("Kimi Code state.json changed while preparing rename; try again".to_string());
    }
    if let Err(first_error) = fs::rename(&temporary, path) {
        let backup = parent.join(format!(
            ".{STATE_FILE}.viewer-{}-{}.bak",
            std::process::id(),
            now_millis()
        ));
        if !path.exists() || fs::rename(path, &backup).is_err() {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "Failed to replace Kimi Code state.json: {first_error}"
            ));
        }
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&temporary);
            return Err(format!("Failed to install Kimi Code state.json: {error}"));
        }
        let _ = fs::remove_file(backup);
    }
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn rename_session_at(root: &Path, path: &Path, name: &str) -> Result<(), String> {
    let title = validate_rename_name(name)?;
    let unit = validate_existing_storage(root, path)?;
    let revision = session_revision_stamp(root, &unit.entry_path())?;
    let state_path = unit.root_path.join(STATE_FILE);
    let (mut state, _) = read_stable_session_wires(root, &unit.entry_path())?;
    let state_revision = file_revision(&state_path)?;
    let object = state
        .as_object_mut()
        .ok_or_else(|| "Kimi Code state.json must be a JSON object".to_string())?;
    object.insert("title".to_string(), Value::String(title.to_string()));
    object.insert("isCustomTitle".to_string(), Value::Bool(true));
    // Kimi CLI normalizes a manually titled session to this exact three-field
    // combination. Leaving an old `replaceable` kind makes the viewer title
    // appear correct while Kimi's resume picker can still replace/show it as
    // an automatic title.
    object.insert("titleKind".to_string(), Value::String("custom".to_string()));
    if session_revision_stamp(root, &unit.entry_path())? != revision {
        return Err("Kimi Code session changed while preparing rename; try again".to_string());
    }
    write_state_atomically(&state_path, &state, &state_revision)
}

fn hard_delete_session_at(root: &Path, path: &Path) -> Result<(), String> {
    let unit = validate_existing_storage(root, path)?;
    let revision = session_revision_stamp(root, &unit.entry_path())?;
    let session_id = state_session_id(&unit.root_path)?;
    let index_before = index_lines(root)?;
    let index_after: Vec<String> = index_before
        .iter()
        .filter(|line| index_line_session_id(line).as_deref() != Some(session_id.as_str()))
        .cloned()
        .collect();
    if session_revision_stamp(root, &unit.entry_path())? != revision {
        return Err("Kimi Code session changed while preparing delete; try again".to_string());
    }
    if index_after.len() != index_before.len() {
        write_index_lines(root, &index_after)?;
    }
    if session_revision_stamp(root, &unit.entry_path())? != revision {
        if index_after.len() != index_before.len() {
            let _ = write_index_lines(root, &index_before);
        }
        return Err("Kimi Code session changed while preparing delete; try again".to_string());
    }
    if let Err(error) = fs::remove_dir_all(&unit.root_path) {
        if index_after.len() != index_before.len() {
            let _ = write_index_lines(root, &index_before);
        }
        return Err(format!(
            "Failed to delete Kimi Code session directory: {error}"
        ));
    }
    if let Some(group) = unit.root_path.parent() {
        let empty = fs::read_dir(group)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if empty {
            let _ = fs::remove_dir(group);
        }
    }
    Ok(())
}

fn session_wire_paths(root: &Path, main_wire: &Path) -> Result<Vec<PathBuf>, String> {
    let unit = validate_existing_storage(root, main_wire)?;
    let agents_dir = unit.root_path.join("agents");
    let mut wires = Vec::new();
    for entry in fs::read_dir(agents_dir)
        .map_err(|error| format!("Failed to read Kimi Code agent wires: {error}"))?
        .flatten()
    {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let wire = entry.path().join("wire.jsonl");
        if is_regular_non_symlink(&wire) {
            wires.push(wire);
        }
    }
    wires.sort();
    Ok(wires)
}

fn revision_value(path: &Path) -> Result<Value, String> {
    let revision = file_revision(path)?;
    let duration = revision
        .modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(serde_json::json!({
        "path": path,
        "size": revision.size,
        "mtimeSeconds": duration.as_secs(),
        "mtimeNanos": duration.subsec_nanos(),
        "device": revision.identity.0,
        "inode": revision.identity.1,
    }))
}

fn session_revision_stamp(root: &Path, main_wire: &Path) -> Result<Value, String> {
    let unit = validate_existing_storage(root, main_wire)?;
    let mut paths = vec![unit.root_path.join(STATE_FILE)];
    paths.extend(session_wire_paths(root, main_wire)?);
    let entries = paths
        .iter()
        .map(|path| revision_value(path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(entries))
}

fn session_cache_revision(root: &Path, main_wire: &Path) -> u64 {
    let Ok(stamp) = session_revision_stamp(root, main_wire) else {
        return mtime_millis(main_wire);
    };
    // FNV-1a is only a cache invalidation token, not a security digest. Unlike
    // a maximum mtime, it also changes when state.json updates behind a newer
    // wire file or when a subagent wire is added.
    serde_json::to_vec(&stamp)
        .unwrap_or_default()
        .into_iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        })
}

fn read_stable_session_wires(
    root: &Path,
    main_wire: &Path,
) -> Result<(Value, WireSnapshot), String> {
    let unit = validate_existing_storage(root, main_wire)?;
    for attempt in 0..SNAPSHOT_RETRIES {
        let wires = session_wire_paths(root, main_wire)?;
        let mut paths = Vec::with_capacity(wires.len() + 1);
        paths.push(unit.root_path.join(STATE_FILE));
        paths.extend(wires.iter().cloned());
        let bytes = read_stable_files(&paths, 0)?;
        if session_wire_paths(root, main_wire)? == wires {
            let state = serde_json::from_slice::<Value>(&bytes[0])
                .map_err(|error| format!("Failed to parse Kimi Code state.json: {error}"))?;
            return Ok((
                state,
                wires.into_iter().zip(bytes.into_iter().skip(1)).collect(),
            ));
        }
        if attempt + 1 < SNAPSHOT_RETRIES {
            std::thread::sleep(SNAPSHOT_RETRY_DELAY);
        }
    }
    Err("Kimi Code session agent list changed while reading; retry later".to_string())
}

fn usage_from_record(event: &Value) -> UsageSummary {
    let usage = event.get("usage").unwrap_or(&Value::Null);
    UsageSummary {
        input_tokens: usage.get("inputOther").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: usage.get("output").and_then(Value::as_u64).unwrap_or(0),
        cache_read_input_tokens: usage
            .get("inputCacheRead")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_creation_input_tokens: usage
            .get("inputCacheCreation")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        ..Default::default()
    }
    .finalize()
}

fn event_time(event: &Value) -> i64 {
    event.get("time").and_then(Value::as_i64).unwrap_or(0)
}

fn turn_for_event(
    session_id: &str,
    project_path: &str,
    user_message: &str,
    timestamp_ms: i64,
    call: CallRecord,
) -> Turn {
    Turn {
        user_message: user_message.to_string(),
        project_path: project_path.to_string(),
        session_id: session_id.to_string(),
        calls: vec![call],
        timestamp_ms,
    }
}

fn is_shell_tool(name: &str) -> bool {
    matches!(name, "Bash" | "Shell" | "Terminal" | "Execute")
}

fn read_turns_from_wire_bytes(
    bytes: &[u8],
    session_id: &str,
    project_path: &str,
    include_user_prompts: bool,
) -> Result<Vec<Turn>, String> {
    let mut turns = Vec::new();
    let mut current_prompt = String::new();

    for line in String::from_utf8_lossy(bytes).lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if include_user_prompts && event_type == "turn.prompt" && is_user_prompt(&event) {
            let prompt = event.get("input").map(prompt_text).unwrap_or_default();
            if !prompt.is_empty() {
                current_prompt = prompt;
            }
            continue;
        }
        if event_type == "usage.record" {
            let usage = usage_from_record(&event);
            let model = event
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let cost = pricing::cost_usd_strict(&model, &usage);
            turns.push(turn_for_event(
                session_id,
                project_path,
                &current_prompt,
                event_time(&event),
                CallRecord {
                    model,
                    usage,
                    cost_usd: cost.unwrap_or(0.0),
                    pricing_missing: cost.is_none(),
                    ..Default::default()
                },
            ));
            continue;
        }
        if event_type != "context.append_loop_event" {
            continue;
        }
        let Some(loop_event) = event.get("event") else {
            continue;
        };
        if loop_event.get("type").and_then(Value::as_str) != Some("tool.call") {
            continue;
        }
        let Some(name) = loop_event.get("name").and_then(Value::as_str) else {
            continue;
        };
        let input = loop_event
            .get("args")
            .map(|args| serde_json::to_string(args).unwrap_or_default())
            .unwrap_or_default();
        let mut call = CallRecord {
            call_count: 0,
            tools: vec![name.to_string()],
            has_agent_spawn: matches!(name, "Agent" | "Task" | "task_spawn"),
            ..Default::default()
        };
        if is_shell_tool(name) {
            if let Some(command) = extract_first_command(&input) {
                call.bash_commands.push(command);
            }
        }
        if let Some(server) = extract_mcp_server(name) {
            call.mcp_servers.push(server);
        }
        turns.push(turn_for_event(
            session_id,
            project_path,
            &current_prompt,
            event_time(&event),
            call,
        ));
    }
    Ok(turns)
}

fn all_usage(root: &Path, main_wire: &Path) -> Result<UsageSummary, String> {
    let mut total = UsageSummary::default();
    let (_, wires) = read_stable_session_wires(root, main_wire)?;
    for (_, bytes) in wires {
        for line in String::from_utf8_lossy(&bytes).lines() {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if event.get("type").and_then(Value::as_str) == Some("usage.record") {
                total.add_assign(&usage_from_record(&event));
            }
        }
    }
    Ok(total)
}

fn latest_main_usage(main_wire: &Path) -> Result<UsageSummary, String> {
    let bytes = read_stable_main_wire(main_wire)?;
    let mut latest = UsageSummary::default();
    for line in String::from_utf8_lossy(&bytes).lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) == Some("usage.record") {
            latest = usage_from_record(&event);
        }
    }
    Ok(latest)
}

fn read_session_turns(root: &Path, main_wire: &Path) -> Result<Vec<Turn>, String> {
    let main_entry = validate_existing_storage(root, main_wire)?.entry_path();
    let (state, wires) = read_stable_session_wires(root, main_wire)?;
    let session_id = nonempty_string(&state, "id")
        .map(str::to_string)
        .unwrap_or_else(|| {
            main_wire
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
    let project_path = nonempty_string(&state, "cwd")
        .unwrap_or_default()
        .to_string();
    let mut turns = Vec::new();
    for (wire, bytes) in wires {
        let is_main = wire == main_entry;
        turns.extend(read_turns_from_wire_bytes(
            &bytes,
            &session_id,
            &project_path,
            is_main,
        )?);
    }
    turns.sort_by_key(|turn| turn.timestamp_ms);
    Ok(turns)
}

impl SessionSource for KimiSource {
    fn name(&self) -> &'static str {
        "kimicode"
    }

    fn list_projects(
        &self,
        _include_codex_internal: bool,
        _include_codex_archived: bool,
    ) -> Result<Vec<ProjectInfo>, String> {
        let mut projects: HashMap<String, ProjectInfo> = HashMap::new();
        for record in discover_session_records(&kimi_home())? {
            let project = projects
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
            project.session_count += 1;
            project.last_modified = project.last_modified.max(record.modified);
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
        let mut records: Vec<KimiSessionRecord> = discover_session_records(&kimi_home())?
            .into_iter()
            .filter(|record| record.cwd == project_key)
            .collect();
        records.sort_by_key(|record| std::cmp::Reverse(record.modified));
        let total = records.len();
        let sessions = records
            .iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>()
            .par_iter()
            .map(|record| session_meta(record))
            .collect();
        Ok(SessionPage { total, sessions })
    }

    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String> {
        let mut messages = parse_main_wire_bytes(&read_stable_main_wire(Path::new(path))?)?;
        crate::util::post_process_session_msgs(&mut messages);
        Ok(messages)
    }

    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String> {
        rename_session_at(&kimi_home(), path, name)
    }

    fn trash_title(&self, path: &Path) -> String {
        let session_dir = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .unwrap_or(path);
        read_state(session_dir)
            .as_ref()
            .and_then(|state| {
                nonempty_string(state, "title").or_else(|| nonempty_string(state, "lastPrompt"))
            })
            .map(clean_title)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Kimi Code session".to_string())
    }

    fn resume_command(&self, session_id: &str, _path: &str) -> AgentCommand {
        AgentCommand::new("kimi").arg("--session").arg(session_id)
    }

    fn new_session_command(&self) -> AgentCommand {
        AgentCommand::new("kimi")
    }

    fn image_src(&self, _block: &Value) -> Option<String> {
        None
    }

    fn usage_summary(&self, path: &str) -> Result<UsageSummary, String> {
        all_usage(&kimi_home(), Path::new(path))
    }

    fn context_usage(&self, path: &str) -> Result<UsageSummary, String> {
        latest_main_usage(Path::new(path))
    }

    fn last_prompt(&self, path: &str) -> Result<Option<String>, String> {
        let session_dir = Path::new(path)
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(|| "Invalid Kimi Code session path".to_string())?;
        if let Some(prompt) = read_state(session_dir)
            .as_ref()
            .and_then(|state| nonempty_string(state, "lastPrompt"))
            .map(truncate_subtitle)
            .filter(|prompt| !prompt.is_empty())
        {
            return Ok(Some(prompt));
        }
        Ok(main_user_prompts(Path::new(path))?
            .into_iter()
            .last()
            .map(|(_, prompt)| truncate_subtitle(&prompt))
            .filter(|prompt| !prompt.is_empty()))
    }

    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String> {
        read_session_turns(&kimi_home(), Path::new(path))
    }

    fn source_mtime(&self, path: &str) -> u64 {
        Path::new(path)
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(|_| session_cache_revision(&kimi_home(), Path::new(path)))
            .unwrap_or_else(|| mtime_millis(Path::new(path)))
    }

    fn contains_text(&self, path: &str, q_lower: &str) -> bool {
        main_user_prompts(Path::new(path))
            .map(|prompts| {
                prompts
                    .into_iter()
                    .any(|(_, prompt)| prompt.to_lowercase().contains(q_lower))
            })
            .unwrap_or(false)
    }

    fn watch_target(&self, path: &str) -> Option<PathBuf> {
        let path = PathBuf::from(path);
        is_regular_non_symlink(&path).then_some(path)
    }

    fn watch_targets(&self, path: &str) -> Vec<PathBuf> {
        let wire = PathBuf::from(path);
        if !is_regular_non_symlink(&wire) {
            return Vec::new();
        }
        let Ok(session_dir) = session_dir_for_main_wire(&wire) else {
            return Vec::new();
        };
        let state = session_dir.join(STATE_FILE);
        if !is_regular_non_symlink(&state) {
            return Vec::new();
        }
        vec![wire, state]
    }

    fn validate_session_path(&self, path: &Path) -> Result<(), String> {
        validate_existing_storage(&kimi_home(), path).map(|_| ())
    }

    fn session_storage_unit(&self, path: &Path) -> Result<SessionStorageUnit, String> {
        validate_existing_storage(&kimi_home(), path)
    }

    fn validate_restore_target(
        &self,
        entry_path: &Path,
        root_path: &Path,
        kind: SessionStorageKind,
    ) -> Result<(), String> {
        if kind != SessionStorageKind::Directory {
            return Err("Kimi Code sessions must be restored as directories".to_string());
        }
        validate_restore_storage(&kimi_home(), entry_path, root_path)
    }

    fn trash_metadata(&self, unit: &SessionStorageUnit) -> Result<Value, String> {
        kimi_index_metadata(&kimi_home(), unit)
    }

    fn before_soft_delete(
        &self,
        unit: &SessionStorageUnit,
        metadata: &Value,
    ) -> Result<(), String> {
        verify_session_revision(&kimi_home(), unit, metadata)
    }

    fn after_soft_delete(
        &self,
        _unit: &SessionStorageUnit,
        metadata: &Value,
    ) -> Result<(), String> {
        remove_index_session(&kimi_home(), metadata_session_id(metadata)?)
    }

    fn after_restore(&self, unit: &SessionStorageUnit, metadata: &Value) -> Result<(), String> {
        restore_index_session(&kimi_home(), unit, metadata)
    }

    fn hard_delete_session(&self, path: &Path) -> Result<(), String> {
        hard_delete_session_at(&kimi_home(), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use crate::util::now_millis;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "kimi-source-test-{name}-{}-{}",
            std::process::id(),
            now_millis()
        ))
    }

    fn create_session(
        root: &Path,
        group: &str,
        id: &str,
        state: Value,
        prompts: &[&str],
    ) -> PathBuf {
        let session_dir = root.join(SESSIONS_DIR).join(group).join(id);
        let main_wire = main_wire_path(&session_dir);
        fs::create_dir_all(main_wire.parent().unwrap()).unwrap();
        fs::write(
            session_dir.join(STATE_FILE),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let body = prompts
            .iter()
            .enumerate()
            .map(|(index, prompt)| {
                serde_json::json!({
                    "type": "turn.prompt",
                    "promptId": format!("prompt-{index}"),
                    "input": [{"type": "text", "text": prompt}],
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&main_wire, format!("{body}\n")).unwrap();
        main_wire
    }

    fn state(id: &str, cwd: &str, title: &str) -> Value {
        serde_json::json!({
            "id": id,
            "cwd": cwd,
            "title": title,
            "lastPrompt": "fallback prompt",
            "createdAt": "2026-08-21T00:00:00Z",
            "updatedAt": "2026-08-21T01:00:00Z",
            "archived": true,
        })
    }

    fn append_events(wire: &Path, events: &[Value]) {
        let body = events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::OpenOptions::new()
            .append(true)
            .open(wire)
            .unwrap()
            .write_all(format!("{body}\n").as_bytes())
            .unwrap();
    }

    fn loop_event(event: Value, time: i64) -> Value {
        serde_json::json!({
            "type": "context.append_loop_event",
            "agentId": "main",
            "event": event,
            "time": time,
        })
    }

    #[test]
    fn discovers_disk_sessions_and_uses_state_metadata() {
        let root = scratch("discover");
        let wire = create_session(
            &root,
            "wd_project_123",
            "session_abc",
            state("actual-id", "/tmp/project", "Session title"),
            &["first prompt", "second prompt"],
        );
        let records = discover_session_records(&root).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.id, "actual-id");
        assert_eq!(record.cwd, "/tmp/project");
        assert_eq!(record.title, "Session title");
        let meta = session_meta(record);
        assert_eq!(meta.path, wire.canonicalize().unwrap().to_string_lossy());
        assert_eq!(meta.message_count, 2);
        assert!(meta.size > 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn index_candidates_are_deduplicated_and_outside_entries_are_ignored() {
        let root = scratch("index");
        create_session(
            &root,
            "wd_project_123",
            "session_abc",
            state("id", "/tmp/project", ""),
            &["first prompt"],
        );
        let index = session_index_path(&root);
        let valid = root
            .join(SESSIONS_DIR)
            .join("wd_project_123")
            .join("session_abc");
        fs::write(
            index,
            format!(
                "{}\n{}\n",
                serde_json::json!({"sessionDir": valid}),
                serde_json::json!({"sessionDir": "/tmp/not-kimi-session"}),
            ),
        )
        .unwrap();
        assert_eq!(discover_session_records(&root).unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_incomplete_or_symlinked_sessions() {
        let root = scratch("unsafe");
        let sessions = sessions_dir(&root);
        fs::create_dir_all(sessions.join("wd_project").join("session_incomplete")).unwrap();
        let valid = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["prompt"],
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = sessions.join("wd_project").join("session_link");
            symlink(
                valid.parent().unwrap().parent().unwrap().parent().unwrap(),
                link,
            )
            .unwrap();
        }
        assert_eq!(discover_session_records(&root).unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validates_directory_storage_and_restore_targets() {
        let root = scratch("storage");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["prompt"],
        );
        let unit = validate_existing_storage(&root, &wire).unwrap();
        assert_eq!(unit.kind, SessionStorageKind::Directory);
        assert_eq!(unit.entry_path(), wire.canonicalize().unwrap());
        assert!(validate_existing_storage(&root, &wire.with_file_name("other.jsonl")).is_err());
        let restore_root = sessions_dir(&root)
            .join("wd_restore")
            .join("session_restore");
        assert!(
            validate_restore_storage(&root, &main_wire_path(&restore_root), &restore_root).is_ok()
        );
        assert!(
            validate_restore_storage(&root, &restore_root.join("wire.jsonl"), &restore_root)
                .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renames_state_without_discarding_unknown_fields() {
        let root = scratch("rename");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            serde_json::json!({
                "id": "id",
                "cwd": "/tmp/project",
                "title": "old title",
                "isCustomTitle": false,
                "titleKind": "replaceable",
                "custom": {"preserved": true},
            }),
            &["prompt"],
        );
        rename_session_at(&root, &wire, "new title").unwrap();
        let renamed =
            read_state(wire.parent().unwrap().parent().unwrap().parent().unwrap()).unwrap();
        assert_eq!(
            renamed.get("title"),
            Some(&Value::String("new title".to_string()))
        );
        assert_eq!(renamed.get("isCustomTitle"), Some(&Value::Bool(true)));
        assert_eq!(
            renamed.get("titleKind"),
            Some(&Value::String("custom".to_string()))
        );
        assert_eq!(
            renamed.pointer("/custom/preserved"),
            Some(&Value::Bool(true))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stable_snapshot_rejects_a_partially_written_state_file() {
        let root = scratch("partial-state");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["prompt"],
        );
        let state_path = wire
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(STATE_FILE);
        fs::write(&state_path, b"{\"id\":").unwrap();
        let error = match KimiSource.read_session(wire.to_str().unwrap()) {
            Ok(_) => panic!("partially written state must not be read"),
            Err(error) => error,
        };
        assert!(error.contains("changed while reading"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_revision_and_watch_targets_include_state_and_subagent_wires() {
        let root = scratch("revision-targets");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["prompt"],
        );
        let unit = validate_existing_storage(&root, &wire).unwrap();
        let before = session_cache_revision(&root, &wire);
        let state_path = unit.root_path.join(STATE_FILE);
        fs::write(
            &state_path,
            serde_json::to_vec(&state("id", "/tmp/project", "renamed elsewhere")).unwrap(),
        )
        .unwrap();
        assert_ne!(before, session_cache_revision(&root, &wire));

        let sub_wire = unit.root_path.join("agents/subagent/wire.jsonl");
        fs::create_dir_all(sub_wire.parent().unwrap()).unwrap();
        fs::write(&sub_wire, "").unwrap();
        let revision = session_revision_stamp(&root, &wire).unwrap();
        assert_eq!(revision.as_array().unwrap().len(), 3);

        let targets = KimiSource.watch_targets(wire.to_str().unwrap());
        assert_eq!(
            targets,
            vec![
                wire.clone(),
                wire.parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join(STATE_FILE),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_soft_delete_when_kimi_changed_the_session_after_metadata_snapshot() {
        let root = scratch("delete-conflict");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["prompt"],
        );
        let unit = validate_existing_storage(&root, &wire).unwrap();
        let metadata = kimi_index_metadata(&root, &unit).unwrap();
        append_events(
            &wire,
            &[serde_json::json!({"type": "usage.record", "usage": {"output": 1}})],
        );
        let error = verify_session_revision(&root, &unit, &metadata).unwrap_err();
        assert!(error.contains("changed while preparing delete"));
        assert!(unit.root_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn removes_and_restores_the_matching_root_index_entry() {
        let root = scratch("index-roundtrip");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("kimi-id", "/tmp/project", "title"),
            &["prompt"],
        );
        let session_dir = wire.parent().unwrap().parent().unwrap().parent().unwrap();
        let saved = serde_json::json!({
            "sessionId": "kimi-id",
            "sessionDir": session_dir,
            "workDir": "/tmp/project",
        })
        .to_string();
        fs::write(
            session_index_path(&root),
            format!(
                "{{\"sessionId\":\"other\",\"sessionDir\":\"/tmp/other\",\"workDir\":\"/tmp/other\"}}\nnot-json\n{saved}\n"
            ),
        )
        .unwrap();
        let unit = validate_existing_storage(&root, &wire).unwrap();
        let metadata = kimi_index_metadata(&root, &unit).unwrap();
        assert_eq!(
            metadata.pointer("/indexEntries/0"),
            Some(&Value::String(saved.clone()))
        );

        remove_index_session(&root, "kimi-id").unwrap();
        let removed = fs::read_to_string(session_index_path(&root)).unwrap();
        assert!(!removed.contains("kimi-id"));
        assert!(removed.contains("not-json"));

        restore_index_session(&root, &unit, &metadata).unwrap();
        let restored = index_lines(&root).unwrap();
        assert_eq!(
            restored
                .iter()
                .filter(|line| index_line_session_id(line).as_deref() == Some("kimi-id"))
                .count(),
            1
        );
        assert!(restored.contains(&saved));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hard_delete_removes_session_directory_and_index_entry() {
        let root = scratch("hard-delete");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("kimi-id", "/tmp/project", "title"),
            &["prompt"],
        );
        let session_dir = wire
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        fs::write(
            session_index_path(&root),
            format!(
                "{}\n{}\n",
                serde_json::json!({"sessionId": "kimi-id", "sessionDir": session_dir, "workDir": "/tmp/project"}),
                serde_json::json!({"sessionId": "other", "sessionDir": "/tmp/other", "workDir": "/tmp/other"}),
            ),
        )
        .unwrap();
        hard_delete_session_at(&root, &wire).unwrap();
        assert!(!session_dir.exists());
        assert!(!sessions_dir(&root).join("wd_project").exists());
        let remaining = fs::read_to_string(session_index_path(&root)).unwrap();
        assert!(!remaining.contains("kimi-id"));
        assert!(remaining.contains("other"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn usage_records_cover_main_and_subagents_without_counting_step_end_twice() {
        let root = scratch("usage");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("kimi-id", "/tmp/project", "title"),
            &["visible prompt"],
        );
        append_events(
            &wire,
            &[
                serde_json::json!({
                    "type": "usage.record", "agentId": "main", "model": "custom/kimi-private",
                    "usage": {"inputOther": 10, "output": 4, "inputCacheRead": 3, "inputCacheCreation": 2}, "time": 10,
                }),
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "turnId": "0", "step": 1, "toolCallId": "bash",
                        "name": "Bash", "args": {"command": "git status --short"},
                    }),
                    11,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "turnId": "0", "step": 1, "toolCallId": "mcp",
                        "name": "mcp__github__list_repos", "args": {},
                    }),
                    12,
                ),
                serde_json::json!({
                    "type": "step.end", "usage": {"inputOther": 999, "output": 999}, "time": 13,
                }),
                serde_json::json!({
                    "type": "usage.record", "agentId": "main", "model": "custom/kimi-private",
                    "usage": {"inputOther": 5, "output": 1, "inputCacheRead": 0, "inputCacheCreation": 0}, "time": 14,
                }),
            ],
        );
        let sub_wire = wire
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("subagent")
            .join("wire.jsonl");
        fs::create_dir_all(sub_wire.parent().unwrap()).unwrap();
        fs::write(
            &sub_wire,
            serde_json::json!({
                "type": "usage.record", "agentId": "subagent", "model": "custom/kimi-private",
                "usage": {"inputOther": 7, "output": 6, "inputCacheRead": 5, "inputCacheCreation": 4}, "time": 15,
            })
            .to_string()
                + "\n",
        )
        .unwrap();

        let summary = all_usage(&root, &wire).unwrap();
        assert_eq!(summary.input_tokens, 22);
        assert_eq!(summary.output_tokens, 11);
        assert_eq!(summary.cache_read_input_tokens, 8);
        assert_eq!(summary.cache_creation_input_tokens, 6);
        assert_eq!(summary.total, 47);
        assert_eq!(latest_main_usage(&wire).unwrap().total, 6);

        let turns = read_session_turns(&root, &wire).unwrap();
        let model_calls: Vec<&CallRecord> = turns
            .iter()
            .flat_map(|turn| &turn.calls)
            .filter(|call| call.call_count == 1)
            .collect();
        assert_eq!(model_calls.len(), 3);
        assert!(model_calls.iter().all(|call| call.pricing_missing));
        assert!(model_calls
            .iter()
            .any(|call| call.usage.total == 19 && call.model == "custom/kimi-private"));
        let tool_calls: Vec<&CallRecord> = turns
            .iter()
            .flat_map(|turn| &turn.calls)
            .filter(|call| call.call_count == 0)
            .collect();
        assert_eq!(tool_calls.len(), 2);
        assert!(tool_calls
            .iter()
            .any(|call| call.bash_commands == ["git"] && call.tools == ["Bash"]));
        assert!(tool_calls.iter().any(
            |call| call.mcp_servers == ["github"] && call.tools == ["mcp__github__list_repos"]
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_configured_and_default_homes() {
        let default_home = PathBuf::from("/home/test");
        let cwd = PathBuf::from("/workspace");
        assert_eq!(
            resolve_kimi_home(None, &default_home, &cwd),
            PathBuf::from("/home/test/.kimi-code")
        );
        assert_eq!(
            resolve_kimi_home(Some(PathBuf::from("custom/kimi")), &default_home, &cwd),
            PathBuf::from("/workspace/custom/kimi")
        );
    }

    #[test]
    fn ignores_known_system_triggered_prompts() {
        let root = scratch("prompt-origin");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["visible prompt"],
        );
        let system_prompt = serde_json::json!({
            "type": "turn.prompt",
            "input": [{"type": "text", "text": "hidden system context"}],
            "origin": {"kind": "system_trigger"},
        });
        fs::OpenOptions::new()
            .append(true)
            .open(&wire)
            .unwrap()
            .write_all(format!("{}\n", system_prompt).as_bytes())
            .unwrap();
        let prompts = main_user_prompts(&wire).unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].1, "visible prompt");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reads_main_wire_in_physical_order_and_matches_concurrent_tools_by_id() {
        let root = scratch("wire-events");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["visible prompt"],
        );
        append_events(
            &wire,
            &[
                loop_event(
                    serde_json::json!({
                        "type": "content.part", "uuid": "think-1", "turnId": "0", "step": 1,
                        "part": {"type": "think", "think": "reasoning"},
                    }),
                    1_787_293_265_000,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "content.part", "uuid": "text-1", "turnId": "0", "step": 1,
                        "part": {"type": "text", "text": "answer"},
                    }),
                    1_787_293_266_000,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "uuid": "call-a-event", "turnId": "0", "step": 1,
                        "toolCallId": "call-a", "name": "Bash", "args": {"command": "first"},
                    }),
                    1_787_293_267_000,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "uuid": "call-b-event", "turnId": "0", "step": 1,
                        "toolCallId": "call-b", "name": "Read", "args": {"path": "second"},
                    }),
                    1_787_293_268_000,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.result", "parentUuid": "call-b-event", "toolCallId": "call-b",
                        "result": {"output": "second result", "isError": false},
                    }),
                    1_787_293_269_000,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.result", "parentUuid": "call-a-event", "toolCallId": "call-a",
                        "result": {"output": "first result", "isError": true},
                    }),
                    1_787_293_270_000,
                ),
            ],
        );
        let messages = read_main_wire(&wire).unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].blocks.len(), 4);
        assert_eq!(messages[1].blocks[0].kind, "thinking");
        assert_eq!(messages[1].blocks[1].kind, "text");
        assert_eq!(messages[1].blocks[2].tool_id.as_deref(), Some("call-a"));
        assert_eq!(messages[1].blocks[3].tool_id.as_deref(), Some("call-b"));
        assert_eq!(messages[2].blocks[0].tool_id.as_deref(), Some("call-b"));
        assert_eq!(messages[2].blocks[0].text.as_deref(), Some("second result"));
        assert!(!messages[2].blocks[0].is_error);
        assert_eq!(messages[3].blocks[0].tool_id.as_deref(), Some("call-a"));
        assert!(messages[3].blocks[0].is_error);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_edit_tool_calls_and_associates_their_results() {
        let root = scratch("edit-tool");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["visible prompt"],
        );
        append_events(
            &wire,
            &[
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "uuid": "edit-event", "turnId": "0", "step": 1,
                        "toolCallId": "edit-call", "name": "Edit",
                        "args": {"path": "src/example.ts", "old_string": "old", "new_string": "new"},
                    }),
                    1,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.result", "toolCallId": "edit-call",
                        "result": {"output": "applied", "isError": false},
                    }),
                    2,
                ),
            ],
        );

        let messages = read_main_wire(&wire).unwrap();
        let edit = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .find(|block| block.tool_name.as_deref() == Some("Edit"))
            .unwrap();
        assert_eq!(edit.kind, "tool_use");
        assert_eq!(edit.tool_id.as_deref(), Some("edit-call"));
        assert!(edit
            .tool_input
            .as_deref()
            .unwrap()
            .contains("src/example.ts"));
        assert_eq!(edit.file_path.as_deref(), Some("src/example.ts"));
        assert_eq!(edit.file_change_type.as_deref(), Some("update"));
        let diff = edit.diff.as_ref().unwrap();
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].lines[0].kind, "del");
        assert_eq!(diff[0].lines[1].kind, "add");
        let result = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .find(|block| block.kind == "tool_result")
            .unwrap();
        assert_eq!(result.tool_id.as_deref(), Some("edit-call"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renders_existing_clipboard_image_paths_in_user_prompts_as_images() {
        let root = scratch("clipboard-image");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("clipboard-2026-08-21-142645-622992C5.png");
        fs::write(&image, []).unwrap();
        let prompt = format!("Please inspect {} and summarize it", image.display());
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &[&prompt],
        );

        let messages = KimiSource.read_session(wire.to_str().unwrap()).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].blocks[0].kind, "image");
        assert_eq!(
            messages[0].blocks[0].image_src.as_deref(),
            Some(image.to_str().unwrap())
        );
        assert_eq!(
            messages[0].blocks[1].text.as_deref(),
            Some("Please inspect [Image #1] and summarize it")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renders_windows_inline_prompt_images_without_indexing_the_data_uri() {
        let root = scratch("windows-inline-image");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["visible prompt"],
        );
        append_events(
            &wire,
            &[serde_json::json!({
                "type": "turn.prompt",
                "promptId": "windows-image",
                "origin": {"kind": "user"},
                "input": [
                    {"type": "text", "text": "hi, "},
                    {"type": "image_url", "imageUrl": {"url": "data:image/png;base64,AQID"}},
                    {"type": "text", "text": " , answer hi"},
                ],
            })],
        );

        let messages = read_main_wire(&wire).unwrap();
        let message = messages.last().unwrap();
        assert_eq!(message.role, "user");
        assert_eq!(message.blocks.len(), 3);
        assert_eq!(message.blocks[0].text.as_deref(), Some("hi,"));
        assert_eq!(message.blocks[1].kind, "image");
        assert_eq!(
            message.blocks[1].image_src.as_deref(),
            Some("data:image/png;base64,AQID")
        );
        assert_eq!(
            message.blocks[1].inline_placeholder.as_deref(),
            Some("[Image #1]")
        );
        assert_eq!(message.blocks[2].text.as_deref(), Some(", answer hi"));

        let prompts = main_user_prompts(&wire).unwrap();
        assert_eq!(prompts.last().unwrap().1, "hi,\n, answer hi");
        assert!(!prompts.last().unwrap().1.contains("AQID"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_recorded_image_tokens_when_windows_prompt_has_inline_images() {
        let root = scratch("windows-image-token");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["visible prompt"],
        );
        append_events(
            &wire,
            &[serde_json::json!({
                "type": "turn.prompt",
                "promptId": "windows-image-token",
                "origin": {"kind": "user"},
                "input": [
                    {"type": "text", "text": "[Image #1] inspect this"},
                    {"type": "image_url", "imageUrl": {"url": "data:image/png;base64,AQID"}},
                ],
            })],
        );

        let messages = KimiSource.read_session(wire.to_str().unwrap()).unwrap();
        let message = messages.last().unwrap();
        assert_eq!(
            message.blocks[0].text.as_deref(),
            Some("[Image #1] inspect this")
        );
        assert_eq!(message.blocks[1].kind, "image");
        assert_eq!(
            message.blocks[1].inline_placeholder.as_deref(),
            Some("[Image #1]")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normalizes_ask_user_question_and_degrades_duplicate_tool_ids() {
        let root = scratch("ask-question");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &["visible prompt"],
        );
        let ask_args = serde_json::json!({
            "questions": [{
                "question": "Choose", "header": "Mode", "multi_select": true,
                "options": [{"label": "A", "description": "first"}, {"label": "B"}],
            }],
        });
        append_events(
            &wire,
            &[
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "uuid": "ask", "turnId": "0", "step": 1,
                        "toolCallId": "ask-id", "name": "AskUserQuestion", "args": ask_args,
                    }),
                    1,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.result", "toolCallId": "ask-id",
                        "result": {"output": "{\\\"answers\\\":{\\\"Choose\\\":\\\"A, B\\\"}}", "isError": false},
                    }),
                    2,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "uuid": "duplicate-one", "turnId": "0", "step": 2,
                        "toolCallId": "duplicate", "name": "Bash", "args": {"command": "one"},
                    }),
                    3,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.call", "uuid": "duplicate-two", "turnId": "0", "step": 2,
                        "toolCallId": "duplicate", "name": "Bash", "args": {"command": "two"},
                    }),
                    4,
                ),
                loop_event(
                    serde_json::json!({
                        "type": "tool.result", "toolCallId": "duplicate",
                        "result": {"output": "must remain unassociated", "isError": false},
                    }),
                    5,
                ),
            ],
        );
        let messages = read_main_wire(&wire).unwrap();
        let ask = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .find(|block| block.tool_name.as_deref() == Some("AskUserQuestion"))
            .unwrap();
        assert_eq!(ask.tool_id.as_deref(), Some("ask-id"));
        let ask_input: Value = serde_json::from_str(ask.tool_input.as_deref().unwrap()).unwrap();
        assert_eq!(
            ask_input.pointer("/questions/0/multiSelect"),
            Some(&Value::Bool(true))
        );
        assert!(ask_input.pointer("/questions/0/multi_select").is_none());
        let duplicate_uses: Vec<&Block> = messages
            .iter()
            .flat_map(|message| &message.blocks)
            .filter(|block| block.tool_name.as_deref() == Some("Bash"))
            .collect();
        assert_eq!(duplicate_uses.len(), 2);
        assert!(duplicate_uses.iter().all(|block| block.tool_id.is_none()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_append_messages_only_when_primary_events_are_absent() {
        let root = scratch("fallback");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("id", "/tmp/project", "title"),
            &[],
        );
        fs::write(
            &wire,
            format!(
                "{}\n{}\n",
                serde_json::json!({
                    "type": "context.append_message", "time": 1,
                    "message": {"id": "user", "role": "user", "origin": {"kind": "user"}, "content": [{"type": "text", "text": "old user"}]},
                }),
                serde_json::json!({
                    "type": "context.append_message", "time": 2,
                    "message": {"id": "system", "role": "user", "origin": {"kind": "injection"}, "content": [{"type": "text", "text": "hidden"}]},
                }),
            ),
        )
        .unwrap();
        let messages = read_main_wire(&wire).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].blocks[0].text.as_deref(), Some("old user"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_commands_use_kimi_session_contracts() {
        let source = KimiSource;
        let resume = source.resume_command("session-123", "ignored");
        assert_eq!(resume.program(), "kimi");
        assert_eq!(resume.args(), &["--session", "session-123"]);

        let new_session = source.new_session_command();
        assert_eq!(new_session.program(), "kimi");
        assert!(new_session.args().is_empty());
    }

    #[test]
    fn resolves_hook_session_id_to_the_validated_main_wire() {
        let root = scratch("hook-wire");
        let wire = create_session(
            &root,
            "wd_project",
            "session_valid",
            state("hook-id", "/tmp/project", "title"),
            &["prompt"],
        );
        assert_eq!(
            find_main_wire_path_at(&root, "hook-id", Some("/tmp/project")),
            Some(wire.canonicalize().unwrap())
        );
        assert_eq!(
            find_main_wire_path_at(&root, "hook-id", Some("/tmp/other")),
            None
        );
        let _ = fs::remove_dir_all(root);
    }
}
