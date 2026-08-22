//! Pi local-session discovery and metadata source (phases 0-1).
//!
//! Pi stores append-only JSONL below a configurable session root. This module
//! intentionally reads only the non-sensitive session records: it never opens
//! Pi auth, model, trust, or credential files.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use chrono::DateTime;
use serde_json::Value;

use super::SessionSource;
use crate::agent_command::AgentCommand;
use crate::stats::pricing;
use crate::stats::types::{CostSource, Turn};
use crate::types::{Block, Msg, PiTreeNode, ProjectInfo, SessionMeta, SessionPage, UsageSummary};
use crate::util::{
    append_jsonl_line, clean_title, home, mtime_millis, now_millis, parse_iso8601_ms,
    validate_rename_name,
};

pub struct PiSource;

const SETTINGS_FILE: &str = "settings.json";
const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
const MAX_HEADER_BYTES: u64 = 64 * 1024;
const MAX_SCAN_FILES: usize = 10_000;
const MAX_SCAN_DEPTH: usize = 12;
const SNAPSHOT_RETRIES: usize = 3;
const SNAPSHOT_RETRY_DELAY: Duration = Duration::from_millis(20);
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
struct PiHeader {
    id: String,
    timestamp: String,
    cwd: String,
}

#[derive(Clone, Debug)]
struct PiSessionRecord {
    path: PathBuf,
    header: PiHeader,
    modified: u64,
    size: u64,
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

fn nonempty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn expand_path(value: &Path, base: &Path) -> PathBuf {
    let raw = value.as_os_str().to_string_lossy();
    if raw == "~" {
        return home();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home().join(rest);
    }
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        base.join(value)
    }
}

fn resolve_pi_agent_dir(configured: Option<PathBuf>, default_home: &Path, cwd: &Path) -> PathBuf {
    configured
        .map(|path| expand_path(&path, cwd))
        .unwrap_or_else(|| default_home.join(".pi").join("agent"))
}

/// Pi's agent data root. A relative environment value is resolved from cwd so
/// it behaves like an explicit command-line path, rather than escaping home.
pub fn pi_agent_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_pi_agent_dir(nonempty_env("PI_CODING_AGENT_DIR"), &home(), &cwd)
}

/// Global Pi settings file used for app-managed extensions. This helper only
/// returns the path; callers must treat the file as untrusted JSON and never
/// read credential-bearing sibling files.
pub fn pi_settings_path() -> PathBuf {
    pi_agent_dir().join(SETTINGS_FILE)
}

/// Absolute path of the Sessions Viewer lifecycle extension installed into
/// Pi's global extension directory.
pub fn pi_status_extension_path() -> PathBuf {
    pi_agent_dir()
        .join("extensions")
        .join("cc-sessions-viewer-turn-status.ts")
}

fn configured_session_dir(agent_dir: &Path) -> Option<PathBuf> {
    let metadata = fs::symlink_metadata(agent_dir.join(SETTINGS_FILE)).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_HEADER_BYTES
    {
        return None;
    }
    let raw = fs::read(agent_dir.join(SETTINGS_FILE)).ok()?;
    let value: Value = serde_json::from_slice(&raw).ok()?;
    let session_dir = value.get("sessionDir")?.as_str()?.trim();
    (!session_dir.is_empty()).then(|| expand_path(Path::new(session_dir), agent_dir))
}

/// Persistent session-root precedence documented by Pi. One-shot `--session-dir`
/// overrides are deliberately not discoverable because Pi does not index them.
pub fn pi_session_root() -> PathBuf {
    let agent_dir = pi_agent_dir();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_pi_session_root(
        nonempty_env("PI_CODING_AGENT_SESSION_DIR"),
        configured_session_dir(&agent_dir),
        &agent_dir,
        &cwd,
    )
}

fn resolve_pi_session_root(
    configured: Option<PathBuf>,
    settings_session_dir: Option<PathBuf>,
    agent_dir: &Path,
    cwd: &Path,
) -> PathBuf {
    configured
        .map(|path| expand_path(&path, cwd))
        .or(settings_session_dir)
        .unwrap_or_else(|| agent_dir.join("sessions"))
}

fn regular_dir(path: &Path) -> Option<PathBuf> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    path.canonicalize().ok()
}

fn regular_file(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (metadata.is_file() && !metadata.file_type().is_symlink()).then_some(metadata)
}

fn is_descendant(path: &Path, root: &Path) -> bool {
    path.starts_with(root) && path != root
}

fn valid_timestamp(value: &str) -> bool {
    DateTime::parse_from_rfc3339(value).is_ok()
}

fn normalize_cwd(value: &str) -> Option<String> {
    let path = Path::new(value.trim());
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(segment) => normalized.push(segment),
            Component::Prefix(_) => return None,
        }
    }
    (normalized != Path::new("/")).then(|| normalized.to_string_lossy().to_string())
}

fn parse_header_value(value: &Value) -> Option<PiHeader> {
    if value.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    let id = value.get("id").and_then(Value::as_str)?.trim();
    let timestamp = value.get("timestamp").and_then(Value::as_str)?.trim();
    let cwd = value.get("cwd").and_then(Value::as_str)?;
    if id.is_empty()
        || id.len() > 256
        || id.chars().any(char::is_control)
        || !valid_timestamp(timestamp)
    {
        return None;
    }
    Some(PiHeader {
        id: id.to_string(),
        timestamp: timestamp.to_string(),
        cwd: normalize_cwd(cwd)?,
    })
}

fn parse_header_bytes(bytes: &[u8]) -> Option<PiHeader> {
    let line = bytes.split(|byte| *byte == b'\n').next()?;
    if line.len() as u64 > MAX_HEADER_BYTES {
        return None;
    }
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let value: Value = serde_json::from_slice(line).ok()?;
    parse_header_value(&value)
}

fn parse_header(path: &Path) -> Option<PiHeader> {
    let file = fs::File::open(path).ok()?;
    use std::io::{BufRead, BufReader, Read};
    let mut reader = BufReader::new(file).take(MAX_HEADER_BYTES);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    parse_header_value(&value)
}

fn scan_sessions(root: &Path) -> Vec<PiSessionRecord> {
    let Some(canonical_root) = regular_dir(root) else {
        return Vec::new();
    };
    let mut pending = vec![(canonical_root.clone(), 0usize)];
    let mut records = Vec::new();
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if records.len() >= MAX_SCAN_FILES {
                return records;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() && depth < MAX_SCAN_DEPTH {
                if let Some(canonical) =
                    regular_dir(&path).filter(|path| is_descendant(path, &canonical_root))
                {
                    pending.push((canonical, depth + 1));
                }
                continue;
            }
            if !file_type.is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }
            let Some(metadata) = regular_file(&path) else {
                continue;
            };
            if metadata.len() > MAX_SESSION_BYTES {
                continue;
            }
            let Ok(canonical) = path.canonicalize() else {
                continue;
            };
            if !is_descendant(&canonical, &canonical_root) {
                continue;
            }
            let Some(header) = parse_header(&canonical) else {
                continue;
            };
            records.push(PiSessionRecord {
                path: canonical,
                header,
                modified: mtime_millis(&path),
                size: metadata.len(),
            });
        }
    }
    records
}

fn file_revision(path: &Path) -> Result<FileRevision, String> {
    let metadata = regular_file(path)
        .ok_or_else(|| format!("Pi session is not a regular file: {}", path.display()))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err("Pi session exceeds the safe size limit".to_string());
    }
    Ok(FileRevision {
        size: metadata.len(),
        modified: metadata.modified().map_err(|e| e.to_string())?,
        identity: file_identity(&metadata),
    })
}

fn stable_bytes(path: &Path) -> Result<Vec<u8>, String> {
    for attempt in 0..SNAPSHOT_RETRIES {
        let before = file_revision(path)?;
        let bytes = fs::read(path).map_err(|e| format!("Failed to read Pi session: {e}"))?;
        let after = file_revision(path)?;
        if before == after {
            return Ok(bytes);
        }
        if attempt + 1 < SNAPSHOT_RETRIES {
            thread::sleep(SNAPSHOT_RETRY_DELAY);
        }
    }
    Err("Pi session changed while reading; retry after its current write completes".to_string())
}

#[derive(Default)]
struct TreeSummary {
    entry_count: usize,
    branch_count: usize,
    message_count: usize,
    title: Option<String>,
}

#[derive(Clone)]
struct PiEntry {
    id: String,
    parent_id: Option<String>,
    value: Value,
}

struct ParsedPi {
    version: u64,
    entries: Vec<PiEntry>,
    by_id: HashMap<String, usize>,
    duplicate_ids: bool,
}

fn parse_entries(bytes: &[u8]) -> Result<ParsedPi, String> {
    let mut version = 3;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut duplicate_ids = false;
    for (ordinal, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_slice(raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("type").and_then(Value::as_str) == Some("session") {
            version = value.get("version").and_then(Value::as_u64).unwrap_or(1);
            continue;
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| (version == 1).then(|| format!("v1:{ordinal}")));
        let Some(id) = id else {
            continue;
        };
        if !seen.insert(id.clone()) {
            duplicate_ids = true;
            continue;
        }
        let parent_id = value
            .get("parentId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| {
                (version == 1)
                    .then(|| entries.last().map(|entry: &PiEntry| entry.id.clone()))
                    .flatten()
            });
        let role = value.pointer("/message/role").and_then(Value::as_str);
        let value = if version == 2 && role == Some("hookMessage") {
            let mut value = value;
            if let Some(message) = value.get_mut("message") {
                if let Some(object) = message.as_object_mut() {
                    object.insert("role".into(), Value::String("custom".into()));
                }
            }
            value
        } else {
            value
        };
        entries.push(PiEntry {
            id,
            parent_id,
            value,
        });
    }
    let by_id = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.id.clone(), index))
        .collect();
    Ok(ParsedPi {
        version,
        entries,
        by_id,
        duplicate_ids,
    })
}

fn entry_lineage<'a>(
    parsed: &'a ParsedPi,
    leaf_id: Option<&str>,
) -> Result<Vec<&'a PiEntry>, String> {
    let leaf = match leaf_id {
        Some(id) => parsed
            .by_id
            .get(id)
            .copied()
            .ok_or_else(|| format!("Pi entry not found: {id}"))?,
        None => parsed
            .entries
            .len()
            .checked_sub(1)
            .ok_or_else(|| "Pi session has no entries".to_string())?,
    };
    let mut result = Vec::new();
    let mut cursor = Some(leaf);
    let mut visited = HashSet::new();
    while let Some(index) = cursor {
        if !visited.insert(index) {
            break;
        }
        let entry = &parsed.entries[index];
        cursor = entry
            .parent_id
            .as_deref()
            .and_then(|id| parsed.by_id.get(id).copied());
        result.push(entry);
    }
    result.reverse();
    Ok(result)
}

fn tree_nodes(parsed: &ParsedPi) -> Vec<PiTreeNode> {
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for entry in &parsed.entries {
        if let Some(parent) = entry.parent_id.as_ref() {
            children.entry(parent.clone()).or_default().push(entry.id.clone());
        }
    }
    parsed.entries.iter().enumerate().map(|(ordinal, entry)| {
        let kind = entry.value.get("type").and_then(Value::as_str).unwrap_or("entry").to_string();
        let kind = if kind == "message" {
            entry.value.pointer("/message/role").and_then(Value::as_str).unwrap_or("message").to_string()
        } else { kind };
        let node_children = children.remove(&entry.id).unwrap_or_default();
        PiTreeNode {
            id: entry.id.clone(),
            parent_id: entry.parent_id.clone(),
            terminal: node_children.is_empty(),
            children: node_children,
            kind,
            timestamp: entry.value.get("timestamp").and_then(Value::as_str).map(str::to_string),
            ordinal,
        }
    }).collect()
}

fn tree_is_unsafe(parsed: &ParsedPi) -> bool {
    if parsed.duplicate_ids {
        return true;
    }
    for entry in &parsed.entries {
        let Some(parent) = entry.parent_id.as_deref() else {
            continue;
        };
        if parent == entry.id || !parsed.by_id.contains_key(parent) {
            return true;
        }
        let mut cursor = Some(entry.id.as_str());
        let mut visited = HashSet::new();
        while let Some(id) = cursor {
            if !visited.insert(id) {
                return true;
            }
            cursor = parsed
                .by_id
                .get(id)
                .and_then(|index| parsed.entries[*index].parent_id.as_deref());
        }
    }
    false
}

fn text_block(text: impl Into<String>) -> Block {
    Block {
        kind: "text".into(),
        text: Some(text.into()),
        ..Default::default()
    }
}

fn image_src(value: &Value) -> Option<String> {
    let url = value
        .pointer("/imageUrl/url")
        .or_else(|| value.pointer("/image_url/url"))
        .or_else(|| value.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty());
    if let Some(url) = url {
        if url.starts_with("data:image/") || url.starts_with("http:") || url.starts_with("https:") {
            return Some(url.to_string());
        }
    }
    let source = value.get("source").unwrap_or(value);
    let media = source
        .get("media_type")
        .or_else(|| source.get("mediaType"))
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    let data = source.get("data").and_then(Value::as_str)?;
    if data.len() > MAX_IMAGE_BYTES {
        return Some("data:image/png;base64,INVALID_IMAGE_TOO_LARGE".into());
    }
    if data.starts_with("data:") {
        Some(data.to_string())
    } else {
        Some(format!("data:{media};base64,{data}"))
    }
}

fn content_blocks(content: &Value, tool_id: Option<&str>, is_error: bool) -> Vec<Block> {
    let items: Vec<Value> = match content {
        Value::Array(items) => items.clone(),
        Value::String(text) => vec![serde_json::json!({"type":"text","text":text})],
        _ => Vec::new(),
    };
    let mut blocks = Vec::new();
    let is_tool_result = tool_id.is_some();
    for item in items {
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    blocks.push(Block {
                        kind: if is_tool_result {
                            "tool_result".into()
                        } else {
                            "text".into()
                        },
                        text: Some(text.into()),
                        tool_id: tool_id.map(str::to_string),
                        is_error,
                        ..Default::default()
                    });
                }
            }
            "thinking" => {
                if let Some(text) = item
                    .get("thinking")
                    .or_else(|| item.get("text"))
                    .and_then(Value::as_str)
                {
                    blocks.push(Block {
                        kind: "thinking".into(),
                        text: Some(text.into()),
                        ..Default::default()
                    });
                }
            }
            "image" | "image_url" => {
                if let Some(src) = image_src(&item) {
                    blocks.push(Block {
                        kind: "image".into(),
                        image_src: Some(src),
                        tool_id: tool_id.map(str::to_string),
                        is_error,
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }
    if blocks.is_empty() && tool_id.is_some() {
        blocks.push(Block {
            kind: "tool_result".into(),
            tool_id: tool_id.map(str::to_string),
            is_error,
            ..Default::default()
        });
    }
    blocks
}

/// Pi expands a slash skill into the complete `<skill ...>SKILL.md</skill>`
/// envelope before persisting the user entry. That envelope is protocol
/// context, not user prose; keep the invocation visible while hiding the
/// injected document body (matching Codex's internal-user filtering).
fn normalize_pi_skill_text(text: &str) -> String {
    let trimmed = text.trim();
    let (prefix, skill_text) = if trimmed.starts_with("<skill ") {
        ("", trimmed)
    } else if let Some(prefix_end) = pi_image_prefix_end(trimmed) {
        let rest = trimmed[prefix_end..].trim_start();
        if !rest.starts_with("<skill ") {
            return text.to_string();
        }
        (trimmed[..prefix_end].trim_end(), rest)
    } else {
        return text.to_string();
    };
    let Some(open_end) = skill_text.find('>') else {
        return text.to_string();
    };
    let opening = &skill_text[..open_end + 1];
    let Some(name_start) = opening.find("name=\"") else {
        return text.to_string();
    };
    let name_start = name_start + "name=\"".len();
    let Some(name_end_rel) = opening[name_start..].find('"') else {
        return text.to_string();
    };
    let name = &opening[name_start..name_start + name_end_rel];
    if name.trim().is_empty() || !opening.starts_with("<skill ") || !opening.contains("location=\"")
    {
        return text.to_string();
    }
    let Some(close_start) = skill_text.rfind("</skill>") else {
        return text.to_string();
    };
    if close_start <= open_end {
        return text.to_string();
    }
    let suffix = skill_text[close_start + "</skill>".len()..].trim();
    let invocation = if suffix.is_empty() {
        format!("/{name}")
    } else {
        // Pi expands the command before persisting it, but the user's original
        // input is a single line (`/skill:name args`). Keep that compact shape
        // after removing the injected document body.
        format!("/{name} {suffix}")
    };
    if prefix.is_empty() {
        invocation
    } else {
        format!("{prefix} {invocation}")
    }
}

/// After generic Pi post-processing, a leading pasted screenshot is represented
/// by `[Image #N]`. Only accept that marker as a prefix; a normal `/path/to/file`
/// remains ordinary user text and is never treated as a slash invocation.
fn pi_image_prefix_end(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut found = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if !text[i..].starts_with("[Image #") {
            break;
        }
        let start = i + "[Image #".len();
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == start || end >= bytes.len() || bytes[end] != b']' {
            return None;
        }
        i = end + 1;
        found = true;
    }
    found.then_some(i)
}

fn normalize_pi_skill_blocks(blocks: &mut [Block]) {
    for block in blocks {
        if block.kind == "text" {
            if let Some(text) = block.text.as_mut() {
                let normalized = normalize_pi_skill_text(text);
                if normalized != *text {
                    *text = normalized;
                }
            }
        }
    }
}

fn entry_to_msgs(entry: &PiEntry) -> Vec<Msg> {
    let timestamp = entry
        .value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    if entry.value.get("type").and_then(Value::as_str) == Some("message") {
        let message = entry.value.get("message").unwrap_or(&Value::Null);
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        match role {
            "user" => {
                let mut blocks =
                    content_blocks(message.get("content").unwrap_or(&Value::Null), None, false);
                normalize_pi_skill_blocks(&mut blocks);
                return vec![Msg {
                    uuid: Some(entry.id.clone()),
                    role: "user".into(),
                    timestamp,
                    sidechain: false,
                    model: None,
                    blocks,
                    meta_kind: None,
                }];
            }
            "assistant" => {
                let mut blocks =
                    content_blocks(message.get("content").unwrap_or(&Value::Null), None, false);
                if let Some(calls) = message.get("content").and_then(Value::as_array) {
                    for call in calls
                        .iter()
                        .filter(|call| call.get("type").and_then(Value::as_str) == Some("toolCall"))
                    {
                        blocks.push(Block {
                            kind: "tool_use".into(),
                            tool_name: call.get("name").and_then(Value::as_str).map(str::to_string),
                            tool_input: call
                                .get("arguments")
                                .map(|v| serde_json::to_string_pretty(v).unwrap_or_default()),
                            tool_id: call.get("id").and_then(Value::as_str).map(str::to_string),
                            ..Default::default()
                        });
                    }
                }
                let model = message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                return vec![Msg {
                    uuid: Some(entry.id.clone()),
                    role: "assistant".into(),
                    timestamp,
                    sidechain: false,
                    model,
                    blocks,
                    meta_kind: None,
                }];
            }
            "toolResult" => {
                let id = message.get("toolCallId").and_then(Value::as_str);
                let blocks = content_blocks(
                    message.get("content").unwrap_or(&Value::Null),
                    id,
                    message
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                );
                return vec![Msg {
                    uuid: Some(entry.id.clone()),
                    role: "assistant".into(),
                    timestamp,
                    sidechain: false,
                    model: None,
                    blocks,
                    meta_kind: None,
                }];
            }
            "bashExecution" => {
                let command = message.get("command").and_then(Value::as_str).unwrap_or("");
                let output = message.get("output").and_then(Value::as_str).unwrap_or("");
                let text = format!("$ {command}\n{output}");
                return vec![Msg {
                    uuid: Some(entry.id.clone()),
                    role: "user".into(),
                    timestamp,
                    sidechain: false,
                    model: None,
                    blocks: vec![text_block(text)],
                    // Pi's standalone bashExecution is tool output, not an AI
                    // reply. Reuse the existing command-output system-note
                    // style instead of falling back to a generic system card.
                    meta_kind: Some("command-output".into()),
                }];
            }
            "custom"
                if message
                    .get("display")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                return vec![Msg {
                    uuid: Some(entry.id.clone()),
                    role: "user".into(),
                    timestamp,
                    sidechain: false,
                    model: None,
                    blocks: content_blocks(
                        message.get("content").unwrap_or(&Value::Null),
                        None,
                        false,
                    ),
                    meta_kind: Some("custom".into()),
                }];
            }
            _ => {}
        }
    }
    if entry.value.get("type").and_then(Value::as_str) == Some("hookMessage") {
        if !entry
            .value
            .get("display")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Vec::new();
        }
        return vec![Msg {
            uuid: Some(entry.id.clone()),
            role: "user".into(),
            timestamp,
            sidechain: false,
            model: None,
            blocks: content_blocks(
                entry.value.get("message").unwrap_or(&Value::Null),
                None,
                false,
            ),
            meta_kind: Some("custom".into()),
        }];
    }
    if entry.value.get("type").and_then(Value::as_str) == Some("custom_message")
        && entry
            .value
            .get("display")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return vec![Msg {
            uuid: Some(entry.id.clone()),
            role: "user".into(),
            timestamp,
            sidechain: false,
            model: None,
            blocks: content_blocks(
                entry.value.get("content").unwrap_or(&Value::Null),
                None,
                false,
            ),
            meta_kind: Some("custom".into()),
        }];
    }
    match entry.value.get("type").and_then(Value::as_str) {
        Some("compaction") | Some("branch_summary") => entry
            .value
            .get("summary")
            .and_then(Value::as_str)
            .map(|text| {
                vec![Msg {
                    uuid: Some(entry.id.clone()),
                    role: "user".into(),
                    timestamp,
                    sidechain: false,
                    model: None,
                    blocks: vec![text_block(text)],
                    meta_kind: Some(
                        entry
                            .value
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("summary")
                            .into(),
                    ),
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Pi stores tool calls and their results as separate message entries. Keep
/// the failure state on the originating call as well as on the result so the
/// UI can apply the same error treatment used by Grok Build.
fn mark_failed_tool_calls(messages: &mut [Msg]) {
    let failed_ids: HashSet<String> = messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter(|block| block.kind == "tool_result" && block.is_error)
        .filter_map(|block| block.tool_id.clone())
        .collect();
    if failed_ids.is_empty() {
        return;
    }
    for block in messages
        .iter_mut()
        .flat_map(|message| message.blocks.iter_mut())
    {
        if block.kind == "tool_use"
            && block
                .tool_id
                .as_deref()
                .is_some_and(|tool_id| failed_ids.contains(tool_id))
        {
            block.is_error = true;
        }
    }
}

fn usage_from_pi(value: &Value) -> UsageSummary {
    let input = value
        .get("input")
        .or_else(|| value.get("inputTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = value
        .get("output")
        .or_else(|| value.get("outputTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = value
        .get("cacheRead")
        .or_else(|| value.get("cacheReadInputTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = value
        .get("cacheWrite")
        .or_else(|| value.get("cacheWriteInputTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning = value
        .get("reasoning")
        .or_else(|| value.get("reasoningOutputTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    UsageSummary {
        input_tokens: input,
        output_tokens: output.saturating_sub(reasoning),
        cache_creation_input_tokens: cache_write,
        cache_creation_1h_input_tokens: 0,
        cache_read_input_tokens: cache_read,
        reasoning_output_tokens: reasoning,
        total: value
            .get("totalTokens")
            .and_then(Value::as_u64)
            .unwrap_or(input + output + cache_write + cache_read + reasoning),
    }
}

fn usage_summary_from_bytes(bytes: &[u8]) -> Result<UsageSummary, String> {
    let parsed = parse_entries(bytes)?;
    let mut summary = UsageSummary::default();
    for entry in &parsed.entries {
        let usage_value =
            if entry.value.pointer("/message/role").and_then(Value::as_str) == Some("assistant") {
                entry.value.pointer("/message/usage")
            } else if matches!(
                entry.value.get("type").and_then(Value::as_str),
                Some("compaction" | "branch_summary")
            ) {
                entry
                    .value
                    .get("usage")
                    .or_else(|| entry.value.pointer("/message/usage"))
            } else {
                None
            };
        let Some(usage_value) = usage_value.filter(|value| value.is_object()) else {
            continue;
        };
        summary.add_assign(&usage_from_pi(usage_value));
    }
    Ok(summary)
}

fn read_turns_from_bytes(bytes: &[u8]) -> Vec<Turn> {
    let Ok(parsed) = parse_entries(bytes) else {
        return Vec::new();
    };
    let session_id = parse_header_bytes(bytes)
        .map(|header| header.id)
        .unwrap_or_default();
    let project_path = parse_header_bytes(bytes)
        .map(|header| header.cwd)
        .unwrap_or_default();
    let mut turns = Vec::new();
    for entry in &parsed.entries {
        let Some(message) = entry.value.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let usage_value = message.get("usage").unwrap_or(&Value::Null);
        let usage = usage_from_pi(usage_value);
        let model = message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let recorded = usage_value
            .pointer("/cost/total")
            .and_then(Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost >= 0.0);
        let pricing_cost = recorded.or_else(|| pricing::cost_usd_strict(model, &usage));
        let mut call = crate::stats::types::CallRecord {
            model: model.to_string(),
            usage,
            cost_usd: pricing_cost.unwrap_or(0.0),
            pricing_missing: pricing_cost.is_none(),
            ..Default::default()
        };
        call.cost_source = match (recorded, pricing_cost) {
            (Some(_), _) => CostSource::Recorded,
            (None, Some(_)) => CostSource::Catalog,
            (None, None) => CostSource::Unpriced,
        };
        call.message_id = Some(format!("pi:{}", entry.id));
        if recorded.is_none() {
            call.pricing_estimated = false;
        }
        if let Some(content) = message.get("content").and_then(Value::as_array) {
            for block in content {
                if block.get("type").and_then(Value::as_str) == Some("toolCall") {
                    if let Some(name) = block.get("name").and_then(Value::as_str) {
                        call.tools.push(name.to_string());
                    }
                }
            }
        }
        turns.push(Turn {
            user_message: String::new(),
            project_path: project_path.clone(),
            session_id: session_id.clone(),
            calls: vec![call],
            timestamp_ms: entry
                .value
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso8601_ms)
                .unwrap_or(0),
        });
    }
    // Pi can persist a separate LLM call while compacting or summarizing a
    // branch. It is not an assistant message and must be counted independently
    // when usage is present, without scanning retainedTail payloads.
    for entry in &parsed.entries {
        if !matches!(
            entry.value.get("type").and_then(Value::as_str),
            Some("compaction" | "branch_summary")
        ) {
            continue;
        }
        let usage_value = entry
            .value
            .get("usage")
            .or_else(|| entry.value.pointer("/message/usage"));
        let Some(usage_value) = usage_value else {
            continue;
        };
        if !usage_value.is_object() {
            continue;
        }
        let usage = usage_from_pi(usage_value);
        let model = entry
            .value
            .get("model")
            .or_else(|| entry.value.pointer("/message/model"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let recorded = usage_value
            .pointer("/cost/total")
            .and_then(Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost >= 0.0);
        let pricing_cost = recorded.or_else(|| pricing::cost_usd_strict(model, &usage));
        let call = crate::stats::types::CallRecord {
            model: model.to_string(),
            message_id: Some(format!("pi:{}", entry.id)),
            usage,
            cost_usd: pricing_cost.unwrap_or(0.0),
            pricing_missing: pricing_cost.is_none(),
            cost_source: match (recorded, pricing_cost) {
                (Some(_), _) => CostSource::Recorded,
                (None, Some(_)) => CostSource::Catalog,
                (None, None) => CostSource::Unpriced,
            },
            ..Default::default()
        };
        turns.push(Turn {
            user_message: String::new(),
            project_path: project_path.clone(),
            session_id: session_id.clone(),
            calls: vec![call],
            timestamp_ms: entry
                .value
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso8601_ms)
                .unwrap_or(0),
        });
    }
    turns
}

fn user_text(entry: &Value) -> Option<String> {
    let message = entry.get("message")?;
    let content = message.get("content")?;
    match content {
        Value::String(value) => Some(value.clone()),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                }
            })
            .find(|text| !clean_title(text).is_empty()),
        _ => None,
    }
}

fn tree_summary(bytes: &[u8]) -> TreeSummary {
    let mut entries: Vec<(String, Option<String>, Value)> = Vec::new();
    let mut version = 3u64;
    for (ordinal, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("session") {
            version = value.get("version").and_then(Value::as_u64).unwrap_or(3);
            continue;
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| (version == 1).then(|| format!("v1:{ordinal}")));
        let Some(id) = id else {
            continue;
        };
        let parent = value
            .get("parentId")
            .and_then(Value::as_str)
            .filter(|parent| !parent.is_empty())
            .map(str::to_string)
            .or_else(|| {
                (version == 1 && !entries.is_empty()).then(|| entries.last().unwrap().0.clone())
            });
        entries.push((id, parent, value));
    }
    let mut ids = HashSet::new();
    let mut children = HashSet::new();
    for (id, parent, _) in &entries {
        if !ids.insert(id.clone()) {
            continue;
        }
        if let Some(parent) = parent {
            children.insert(parent.clone());
        }
    }
    let leaf = entries
        .iter()
        .rev()
        .find(|(id, _, _)| ids.contains(id))
        .map(|(id, _, _)| id.clone());
    let index: HashMap<&str, &(String, Option<String>, Value)> = entries
        .iter()
        .filter(|(id, _, _)| ids.contains(id))
        .map(|entry| (entry.0.as_str(), entry))
        .collect();
    let mut lineage = Vec::new();
    let mut cursor = leaf.as_deref();
    let mut visited = HashSet::new();
    while let Some(id) = cursor {
        if !visited.insert(id) {
            break;
        }
        let Some(entry) = index.get(id) else {
            break;
        };
        lineage.push(*entry);
        cursor = entry.1.as_deref();
    }
    lineage.reverse();
    let message_count = lineage
        .iter()
        .filter(|(_, _, entry)| {
            entry.get("type").and_then(Value::as_str) == Some("message")
                && matches!(
                    entry
                        .get("message")
                        .and_then(|message| message.get("role"))
                        .and_then(Value::as_str),
                    Some("user" | "assistant")
                )
        })
        .count();
    let latest_name = entries
        .iter()
        .filter_map(|(_, _, entry)| {
            (entry.get("type").and_then(Value::as_str) == Some("session_info"))
                .then(|| {
                    entry
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .flatten()
        })
        .next_back();
    let fallback_title = lineage.iter().find_map(|(_, _, entry)| {
        (entry.get("type").and_then(Value::as_str) == Some("message")
            && entry
                .get("message")
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                == Some("user"))
        .then(|| user_text(entry))
        .flatten()
        .map(|text| clean_title(&normalize_pi_skill_text(&text)))
        .filter(|text| !text.is_empty())
    });
    let title = match latest_name {
        Some(name) if name.trim().is_empty() => None,
        Some(name) => Some(name),
        None => fallback_title,
    };
    TreeSummary {
        entry_count: ids.len(),
        branch_count: ids.iter().filter(|id| !children.contains(*id)).count(),
        message_count,
        title,
    }
}

fn session_meta(record: &PiSessionRecord) -> SessionMeta {
    let summary = stable_bytes(&record.path)
        .map(|bytes| tree_summary(&bytes))
        .unwrap_or_default();
    let file_name = record
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let title = summary
        .title
        .unwrap_or_else(|| file_name.trim_end_matches(".jsonl").to_string());
    SessionMeta {
        id: record.header.id.clone(),
        file_name,
        path: record.path.to_string_lossy().to_string(),
        title,
        cwd: Some(record.header.cwd.clone()),
        created: Some(record.header.timestamp.clone()),
        modified: record.modified,
        size: record.size,
        message_count: summary.message_count,
        pi_branch_count: Some(summary.branch_count),
        pi_entry_count: Some(summary.entry_count),
        codex_app_list_rank: None,
        codex_app_list_scanned: 0,
        codex_app_first_page_size: 0,
        codex_app_first_page_position: 0,
        codex_internal: false,
        codex_archived: false,
    }
}

fn pi_chat_slash_commands(cwd: &str) -> Vec<crate::types::SlashCommand> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let project = Path::new(cwd);
    let project_skill_dirs = [project.join(".pi/skills"), project.join(".agents/skills")];
    for dir in project_skill_dirs {
        super::claude::scan_skills_dir(&dir, "project", None, None, &mut out, &mut seen);
    }
    let user_skill_dirs = [pi_agent_dir().join("skills"), home().join(".agents/skills")];
    for dir in user_skill_dirs {
        super::claude::scan_skills_dir(&dir, "user", None, None, &mut out, &mut seen);
    }
    out.sort_by(|a, b| {
        a.origin
            .cmp(&b.origin)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

impl SessionSource for PiSource {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn list_projects(&self, _: bool, _: bool) -> Result<Vec<ProjectInfo>, String> {
        let mut projects = HashMap::new();
        for record in scan_sessions(&pi_session_root()) {
            let project = projects
                .entry(record.header.cwd.clone())
                .or_insert_with(|| ProjectInfo {
                    dir_name: record.header.cwd.clone(),
                    display_path: record.header.cwd.clone(),
                    session_count: 0,
                    last_modified: 0,
                    exists: Path::new(&record.header.cwd).is_dir(),
                    bookmarked: false,
                    parent_dir_name: None,
                    worktree_name: None,
                });
            project.session_count += 1;
            project.last_modified = project.last_modified.max(record.modified);
        }
        let mut projects: Vec<_> = projects.into_values().collect();
        projects.sort_by_key(|project| std::cmp::Reverse(project.last_modified));
        Ok(projects)
    }

    fn list_sessions(
        &self,
        project_key: &str,
        offset: usize,
        limit: usize,
        _: bool,
        _: bool,
    ) -> Result<SessionPage, String> {
        let mut records: Vec<_> = scan_sessions(&pi_session_root())
            .into_iter()
            .filter(|record| record.header.cwd == project_key)
            .collect();
        records.sort_by_key(|record| std::cmp::Reverse(record.modified));
        let total = records.len();
        let sessions = records
            .iter()
            .skip(offset)
            .take(limit)
            .map(session_meta)
            .collect();
        Ok(SessionPage { total, sessions })
    }

    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String> {
        // The watcher/search APIs only have the physical path and need the
        // default leaf. Explicit branch reads continue to use read_session_at.
        self.read_session_at(path, None)
    }
    fn read_session_at(&self, path: &str, leaf_id: Option<&str>) -> Result<Vec<Msg>, String> {
        let bytes = stable_bytes(Path::new(path))?;
        let parsed = parse_entries(&bytes)?;
        let lineage = entry_lineage(&parsed, leaf_id)?;
        let mut messages: Vec<Msg> = lineage.into_iter().flat_map(entry_to_msgs).collect();
        mark_failed_tool_calls(&mut messages);
        // Pi persists pasted screenshots as clipboard-*.png paths inside user
        // text. Reuse the shared attachment pass used by Kimi/Claude/Codex so
        // those paths become image blocks and [Image #N] placeholders.
        crate::util::post_process_session_msgs(&mut messages);
        // Image extraction can move a leading absolute clipboard path out of
        // the text block. Run skill normalization once more so an image + skill
        // prompt follows the same compact rendering as a plain skill prompt.
        for message in &mut messages {
            normalize_pi_skill_blocks(&mut message.blocks);
        }
        Ok(messages)
    }
    fn session_tree(&self, path: &str) -> Result<Vec<PiTreeNode>, String> {
        let parsed = parse_entries(&stable_bytes(Path::new(path))?)?;
        Ok(tree_nodes(&parsed))
    }
    fn session_export_json(&self, path: &str, leaf_id: Option<&str>) -> Result<String, String> {
        let bytes = stable_bytes(Path::new(path))?;
        let parsed = parse_entries(&bytes)?;
        let selected_leaf = leaf_id
            .map(|id| {
                parsed.by_id.contains_key(id)
                    .then_some(id.to_string())
                    .ok_or_else(|| format!("Pi entry not found: {id}"))
            })
            .transpose()?
            .or_else(|| parsed.entries.last().map(|entry| entry.id.clone()));
        let mut entries = Vec::new();
        let mut header = Value::Null;
        for raw in bytes.split(|byte| *byte == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.is_empty() { continue; }
            let Ok(value) = serde_json::from_slice::<Value>(raw) else { continue; };
            if value.get("type").and_then(Value::as_str) == Some("session") {
                header = value;
            } else if value.get("id").and_then(Value::as_str).is_some() {
                entries.push(value);
            }
        }
        serde_json::to_string_pretty(&serde_json::json!({
            "__type": "cc-session-viewer-pi-export",
            "schemaVersion": 1,
            "piVersion": parsed.version,
            "rendererVersion": 1,
            "selectedLeafId": selected_leaf,
            "header": header,
            "entries": entries,
        })).map_err(|e| format!("Pi export serialization failed: {e}"))
    }
    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String> {
        let name = validate_rename_name(name)?;
        validate_pi_session_path(path)?;
        let before = file_revision(path)?;
        let bytes = stable_bytes(path)?;
        let parsed = parse_entries(&bytes)?;
        if parsed.version == 1 {
            return Err("Pi v1 sessions cannot be renamed before native migration".into());
        }
        if tree_is_unsafe(&parsed) {
            return Err("Pi session tree is structurally unsafe; rename disabled".into());
        }
        let parent_id = parsed.entries.last().map(|entry| entry.id.clone());
        let id = unique_entry_id(&parsed.by_id);
        let line = serde_json::json!({
            "type": "session_info", "id": id, "parentId": parent_id,
            "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true), "name": name
        }).to_string();
        if file_revision(path)? != before {
            return Err("Pi session changed before rename; retry after Pi exits".into());
        }
        append_jsonl_line(path, &line)?;
        if file_revision(path)?.size <= before.size {
            return Err("Pi rename append was not observed".into());
        }
        Ok(())
    }
    fn trash_title(&self, path: &Path) -> String {
        parse_header(path)
            .map(|header| header.id)
            .unwrap_or_else(|| "Pi session".to_string())
    }
    fn resume_command(&self, _: &str, path: &str) -> AgentCommand {
        AgentCommand::new("pi").arg("--session").arg(path)
    }
    fn new_session_command(&self) -> AgentCommand {
        AgentCommand::new("pi")
    }
    fn chat_slash_commands(&self, cwd: &str) -> Vec<crate::types::SlashCommand> {
        pi_chat_slash_commands(cwd)
    }
    fn image_src(&self, _: &Value) -> Option<String> {
        None
    }
    fn usage_summary(&self, path: &str) -> Result<UsageSummary, String> {
        usage_summary_from_bytes(&stable_bytes(Path::new(path))?)
    }
    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String> {
        let path = Path::new(path);
        Ok(read_turns_from_bytes(&stable_bytes(path)?))
    }
    fn validate_session_path(&self, path: &Path) -> Result<(), String> {
        validate_pi_session_path(path)
    }
    fn validate_restore_target(
        &self,
        entry_path: &Path,
        root_path: &Path,
        kind: super::SessionStorageKind,
    ) -> Result<(), String> {
        if kind != super::SessionStorageKind::File || entry_path != root_path {
            return Err("Invalid Pi restore target".into());
        }
        if !entry_path.is_absolute()
            || entry_path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
        {
            return Err("Pi restore target must be an absolute .jsonl path".into());
        }
        let root = regular_dir(&pi_session_root())
            .ok_or_else(|| "Pi session root does not exist".to_string())?;
        let parent = entry_path
            .parent()
            .ok_or_else(|| "Pi restore target has no parent".to_string())?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|_| "Pi restore target parent cannot be resolved".to_string())?;
        if !is_descendant(&canonical_parent, &root) && canonical_parent != root {
            return Err("Pi restore target is outside the configured session root".into());
        }
        if let Ok(metadata) = fs::symlink_metadata(entry_path) {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err("Pi restore target is not a regular file".into());
            }
        }
        Ok(())
    }
}

fn unique_entry_id(ids: &HashMap<String, usize>) -> String {
    let seed = now_millis() as u32;
    for offset in 0..u32::MAX {
        let id = format!("{:08x}", seed.wrapping_add(offset));
        if !ids.contains_key(&id) {
            return id;
        }
    }
    unreachable!()
}

fn validate_pi_session_path(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Pi session file does not exist".to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
    {
        return Err("Pi session path must be a regular .jsonl file".into());
    }
    let root = regular_dir(&pi_session_root())
        .ok_or_else(|| "Pi session root does not exist".to_string())?;
    let canonical = path
        .canonicalize()
        .map_err(|_| "Pi session path cannot be resolved".to_string())?;
    if !is_descendant(&canonical, &root) {
        return Err("Pi session path is outside the configured session root".into());
    }
    if parse_header(&canonical).is_none() {
        return Err("Pi session header is invalid".into());
    }
    Ok(())
}

/// Terminal launch guard: Pi must run from the cwd recorded in its session
/// header. This prevents resuming a path under a different project directory.
pub fn validate_terminal_cwd(path: &Path, cwd: &Path) -> Result<(), String> {
    validate_pi_session_path(path)?;
    let header = parse_header(
        &path
            .canonicalize()
            .map_err(|_| "Pi session path cannot be resolved".to_string())?,
    )
    .ok_or_else(|| "Pi session header is invalid".to_string())?;
    let actual = cwd
        .canonicalize()
        .map_err(|_| "Pi terminal cwd cannot be resolved".to_string())?;
    let expected = Path::new(&header.cwd)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&header.cwd));
    if actual != expected {
        return Err("Pi terminal cwd does not match the session header cwd".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("pi-source-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_session(root: &Path, file: &str, lines: &[Value]) -> PathBuf {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&path, raw).unwrap();
        path
    }

    fn header(version: u64, id: &str, cwd: &str) -> Value {
        serde_json::json!({"type":"session","version":version,"id":id,"timestamp":"2026-08-22T00:00:00.000Z","cwd":cwd})
    }

    #[test]
    fn usage_summary_reads_pi_persisted_assistant_usage() {
        let root = temp_root("usage-summary");
        let path = write_session(
            &root,
            "session.jsonl",
            &[
                header(3, "session", "/tmp/project"),
                serde_json::json!({
                    "type": "message",
                    "id": "assistant-1",
                    "parentId": null,
                    "message": {
                        "role": "assistant",
                        "model": "deepseek-v4-flash",
                        "usage": {
                            "input": 10,
                            "output": 5,
                            "cacheRead": 2,
                            "cacheWrite": 3,
                            "reasoning": 1,
                            "totalTokens": 21
                        }
                    }
                }),
            ],
        );

        let usage = PiSource.usage_summary(path.to_str().unwrap()).unwrap();
        assert_eq!(usage.total, 21);
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 4);
        assert_eq!(usage.cache_read_input_tokens, 2);
        assert_eq!(usage.cache_creation_input_tokens, 3);
        assert_eq!(usage.reasoning_output_tokens, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn session_tree_returns_children_and_terminal_leaves() {
        let root = temp_root("tree-nodes");
        let path = write_session(
            &root,
            "session.jsonl",
            &[
                header(3, "session", "/tmp/project"),
                serde_json::json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"A"}}),
                serde_json::json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"B"}]}}),
                serde_json::json!({"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"C"}]}}),
            ],
        );
        let parsed = parse_entries(&stable_bytes(&path).unwrap()).unwrap();
        let nodes = tree_nodes(&parsed);
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].children, vec!["b".to_string(), "c".to_string()]);
        assert!(!nodes[0].terminal);
        assert!(nodes[1].terminal && nodes[2].terminal);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn usage_summary_accumulates_assistant_and_compaction_usage() {
        let root = temp_root("usage-summary-aggregate");
        let path = write_session(
            &root,
            "session.jsonl",
            &[
                header(3, "session", "/tmp/project"),
                serde_json::json!({
                    "type": "message", "id": "assistant-1", "parentId": null,
                    "message": {"role": "assistant", "usage": {"input": 2, "output": 3, "totalTokens": 5}}
                }),
                serde_json::json!({
                    "type": "compaction", "id": "compaction-1", "parentId": "assistant-1",
                    "usage": {"input": 7, "output": 1, "totalTokens": 8}
                }),
            ],
        );

        let usage = PiSource.usage_summary(path.to_str().unwrap()).unwrap();
        assert_eq!(usage.total, 13);
        assert_eq!(usage.input_tokens, 9);
        assert_eq!(usage.output_tokens, 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hides_pi_skill_document_and_keeps_invocation() {
        let skill = r#"<skill name="git-push" location="/tmp/project/.agents/skills/git-push/SKILL.md">
References are relative to /tmp/project/.agents/skills/git-push.

# Git Push

## Workflow

1. Pull first
</skill>"#;
        assert_eq!(normalize_pi_skill_text(skill), "/git-push");
        assert_eq!(
            normalize_pi_skill_text("<skill name=\"git-push\">partial"),
            "<skill name=\"git-push\">partial"
        );
        assert_eq!(
            normalize_pi_skill_text("用户输入 <skill name=\"git-push\">文本</skill>"),
            "用户输入 <skill name=\"git-push\">文本</skill>"
        );

        let mut blocks = vec![text_block(skill)];
        normalize_pi_skill_blocks(&mut blocks);
        assert_eq!(blocks[0].text.as_deref(), Some("/git-push"));
    }

    #[test]
    fn keeps_pi_skill_arguments_but_hides_expanded_document() {
        let text = r#"<skill name="smux" location="/tmp/project/.agents/skills/smux/SKILL.md">
References are relative to /tmp/project/.agents/skills/smux.

# smux

Use tmux-bridge for pane control.
</skill>

告诉我这个skill作用即可"#;
        assert_eq!(
            normalize_pi_skill_text(text),
            "/smux 告诉我这个skill作用即可"
        );
    }

    #[test]
    fn preserves_strict_image_prefix_when_normalizing_pi_skill() {
        let text = r#"[Image #1] <skill name="smux" location="/tmp/project/.agents/skills/smux/SKILL.md">
References are relative to /tmp/project/.agents/skills/smux.

# smux

Use tmux-bridge for pane control.
</skill>

说明这张图和 skill 的关系"#;
        assert_eq!(
            normalize_pi_skill_text(text),
            "[Image #1] /smux 说明这张图和 skill 的关系"
        );
        assert_eq!(
            normalize_pi_skill_text(
                "[Image #1] [Image #2] <skill name=\"smux\" location=\"/tmp/smux/SKILL.md\">\ndocs\n</skill>"
            ),
            "[Image #1] [Image #2] /smux"
        );
    }

    #[test]
    fn does_not_treat_absolute_paths_as_image_or_skill_prefixes() {
        let text = "/Users/wuchao/Downloads/clipboard.png\n<skill name=\"smux\" location=\"/tmp/smux/SKILL.md\">\ndocs\n</skill>";
        assert_eq!(normalize_pi_skill_text(text), text);
        assert_eq!(
            normalize_pi_skill_text(
                "[Image #x] <skill name=\"smux\" location=\"/tmp/smux/SKILL.md\">\ndocs\n</skill>"
            ),
            "[Image #x] <skill name=\"smux\" location=\"/tmp/smux/SKILL.md\">\ndocs\n</skill>"
        );
    }

    #[test]
    fn renders_pi_skill_invocations_without_document_body() {
        let root = temp_root("skill-invocations");
        let skill = |id: &str, parent_id: &str| {
            serde_json::json!({
                "type":"message",
                "id":id,
                "parentId":parent_id,
                "message":{"role":"user","content":[{"type":"text","text":"<skill name=\"git-push\" location=\"/tmp/project/.agents/skills/git-push/SKILL.md\">\n# Git Push\n\n## Workflow\n\n1. Pull first\n</skill>"}]}
            })
        };
        let path = write_session(
            &root,
            "skill.jsonl",
            &[
                header(3, "skill-session", "/tmp/project"),
                skill("skill-1", ""),
                serde_json::json!({"type":"message","id":"assistant-1","parentId":"skill-1","message":{"role":"assistant","content":"done"}}),
                skill("skill-2", "assistant-1"),
            ],
        );
        let messages = PiSource
            .read_session_at(path.to_str().unwrap(), None)
            .unwrap();
        let invocations: Vec<_> = messages
            .iter()
            .filter(|message| message.role == "user")
            .flat_map(|message| message.blocks.iter())
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.as_deref())
            .collect();
        assert_eq!(invocations, vec!["/git-push", "/git-push"]);
        assert!(messages.iter().all(|message| {
            message
                .blocks
                .iter()
                .filter_map(|block| block.text.as_deref())
                .all(|text| !text.contains("## Workflow"))
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discovers_pi_skill_description_for_command_hover() {
        let root = temp_root("skill-scan");
        let skill_dir = root.join(".agents/skills/git-push");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: git-push\ndescription: Pull, commit and push changes.\n---\n\n# Git Push\n",
        )
        .unwrap();
        let commands = pi_chat_slash_commands(root.to_str().unwrap());
        let command = commands.iter().find(|command| command.name == "git-push");
        assert!(command.is_some(), "git-push skill was not discovered");
        let command = command.unwrap();
        assert_eq!(command.description, "Pull, commit and push changes.");
        assert_eq!(command.kind, "skill");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discovers_only_regular_pi_jsonl_and_groups_by_header_cwd() {
        let root = temp_root("discover");
        write_session(
            &root,
            "encoded-directory/a.jsonl",
            &[header(3, "session-a", "/tmp/pi-project/./child/..")],
        );
        fs::write(root.join("bad.jsonl"), "{bad").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            root.join("encoded-directory/a.jsonl"),
            root.join("link.jsonl"),
        )
        .unwrap();
        let records = scan_sessions(&root);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].header.cwd, "/tmp/pi-project");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn data_root_precedence_honors_environment_then_settings_then_default() {
        let home = PathBuf::from("/home/tester");
        let cwd = PathBuf::from("/work/current");
        let agent = PathBuf::from("/work/agent");
        assert_eq!(
            resolve_pi_agent_dir(Some(PathBuf::from("agent-data")), &home, &cwd),
            cwd.join("agent-data")
        );
        assert_eq!(
            resolve_pi_agent_dir(None, &home, &cwd),
            home.join(".pi/agent")
        );
        assert_eq!(
            resolve_pi_session_root(
                Some(PathBuf::from("/override")),
                Some(PathBuf::from("/configured")),
                &agent,
                &cwd
            ),
            PathBuf::from("/override")
        );
        assert_eq!(
            resolve_pi_session_root(None, Some(PathBuf::from("/configured")), &agent, &cwd),
            PathBuf::from("/configured")
        );
        assert_eq!(
            resolve_pi_session_root(None, None, &agent, &cwd),
            agent.join("sessions")
        );
    }

    #[test]
    fn metadata_uses_latest_session_name_and_physical_last_leaf_lineage() {
        let root = temp_root("metadata");
        let file = write_session(
            &root,
            "nested/a.jsonl",
            &[
                header(3, "session-a", "/tmp/project"),
                serde_json::json!({"type":"message","id":"u","timestamp":"2026-08-22T00:01:00Z","message":{"role":"user","content":"Visible title"}}),
                serde_json::json!({"type":"message","id":"a","parentId":"u","timestamp":"2026-08-22T00:02:00Z","message":{"role":"assistant","content":"answer"}}),
                serde_json::json!({"type":"message","id":"tool","parentId":"a","timestamp":"2026-08-22T00:03:00Z","message":{"role":"toolResult","content":"tool"}}),
                serde_json::json!({"type":"session_info","id":"info","parentId":"tool","timestamp":"2026-08-22T00:04:00Z","name":"Renamed"}),
            ],
        );
        let record = scan_sessions(&root).pop().unwrap();
        let meta = session_meta(&record);
        assert_eq!(meta.path, file.canonicalize().unwrap().to_string_lossy());
        assert_eq!(meta.title, "Renamed");
        assert_eq!(meta.message_count, 2);
        assert_eq!(meta.pi_entry_count, Some(4));
        assert_eq!(meta.pi_branch_count, Some(1));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_empty_latest_session_name_explicitly_clears_the_title() {
        let root = temp_root("clear-name");
        write_session(
            &root,
            "a.jsonl",
            &[
                header(3, "session-a", "/tmp/project"),
                serde_json::json!({"type":"message","id":"u","message":{"role":"user","content":"Fallback title"}}),
                serde_json::json!({"type":"session_info","id":"clear","parentId":"u","name":""}),
            ],
        );
        let record = scan_sessions(&root).pop().unwrap();
        assert_eq!(session_meta(&record).title, "a");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v1_uses_ephemeral_ordinal_ids_and_v2_hook_message_is_not_counted_as_chat() {
        let root = temp_root("versions");
        write_session(
            &root,
            "v1.jsonl",
            &[
                header(1, "v1", "/tmp/v1"),
                serde_json::json!({"type":"message","message":{"role":"user","content":"first"}}),
                serde_json::json!({"type":"message","message":{"role":"assistant","content":"second"}}),
            ],
        );
        write_session(
            &root,
            "v2.jsonl",
            &[
                header(2, "v2", "/tmp/v2"),
                serde_json::json!({"type":"hookMessage","id":"hook","message":"legacy"}),
            ],
        );
        let mut records = scan_sessions(&root);
        records.sort_by_key(|record| record.header.id.clone());
        let v1 = session_meta(&records[0]);
        let v2 = session_meta(&records[1]);
        assert_eq!(v1.message_count, 2);
        assert_eq!(v1.pi_entry_count, Some(2));
        assert_eq!(v2.message_count, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn read_session_follows_default_physical_leaf_and_explicit_leaf_without_writing() {
        let root = temp_root("read-tree");
        let path = write_session(
            &root,
            "tree.jsonl",
            &[
                header(3, "tree", "/tmp/project"),
                serde_json::json!({"type":"message","id":"u","message":{"role":"user","content":"question"}}),
                serde_json::json!({"type":"message","id":"a","parentId":"u","message":{"role":"assistant","content":[{"type":"text","text":"answer"},{"type":"toolCall","id":"call","name":"bash","arguments":{"command":"echo safe"}}]}}),
                serde_json::json!({"type":"message","id":"r","parentId":"a","message":{"role":"toolResult","toolCallId":"call","isError":true,"content":[{"type":"text","text":"failed"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AQ=="}}]}}),
                serde_json::json!({"type":"message","id":"branch","parentId":"u","message":{"role":"assistant","content":"other"}}),
            ],
        );
        let source = PiSource;
        let default_msgs = source
            .read_session_at(path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(
            default_msgs
                .last()
                .and_then(|msg| msg.blocks.first())
                .and_then(|block| block.text.as_deref()),
            Some("other")
        );
        let branch_msgs = source
            .read_session_at(path.to_str().unwrap(), Some("r"))
            .unwrap();
        assert_eq!(branch_msgs.len(), 3);
        assert!(branch_msgs.iter().any(|msg| msg
            .blocks
            .iter()
            .any(|block| block.kind == "tool_use" && block.is_error)));
        assert!(branch_msgs.iter().any(|msg| msg
            .blocks
            .iter()
            .any(|block| block.kind == "tool_result" && block.is_error)));
        assert_eq!(
            source.read_session(path.to_str().unwrap()).unwrap().len(),
            2
        );
        let before = fs::read(&path).unwrap();
        assert!(source
            .read_session_at(path.to_str().unwrap(), Some("missing"))
            .is_err());
        assert_eq!(before, fs::read(&path).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renders_pi_bash_execution_as_command_output_note() {
        let entry = PiEntry {
            id: "bash".into(),
            parent_id: None,
            value: serde_json::json!({
                "type":"message",
                "id":"bash",
                "message":{"role":"bashExecution","command":"git pull","output":"Already up to date.","exitCode":0}
            }),
        };
        let messages = entry_to_msgs(&entry);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].meta_kind.as_deref(), Some("command-output"));
        assert_eq!(
            messages[0].blocks[0].text.as_deref(),
            Some("$ git pull\nAlready up to date.")
        );
    }

    #[test]
    fn custom_message_respects_display_flag_and_read_turns_uses_header_id() {
        let root = temp_root("custom-message");
        let path = write_session(
            &root,
            "custom.jsonl",
            &[
                header(3, "session-header", "/tmp/project"),
                serde_json::json!({"type":"custom_message","id":"hidden","display":false,"content":"do not render"}),
                serde_json::json!({"type":"custom_message","id":"shown","display":true,"content":"extension note"}),
                serde_json::json!({"type":"message","id":"a","parentId":"shown","message":{"role":"assistant","model":"unknown","content":"done","usage":{"input":1,"output":2,"totalTokens":3,"cost":{"total":0.2}}}}),
            ],
        );
        let source = PiSource;
        let msgs = source
            .read_session_at(path.to_str().unwrap(), None)
            .unwrap();
        assert!(msgs.iter().all(|msg| {
            !msg.blocks
                .iter()
                .any(|block| block.text.as_deref() == Some("do not render"))
        }));
        assert!(msgs.iter().any(|msg| {
            msg.meta_kind.as_deref() == Some("custom")
                && msg
                    .blocks
                    .iter()
                    .any(|block| block.text.as_deref() == Some("extension note"))
        }));
        let turns = source.read_turns(path.to_str().unwrap()).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].session_id, "session-header");
        assert_eq!(turns[0].calls[0].cost_source, CostSource::Recorded);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renders_pi_clipboard_image_paths_like_kimi() {
        let root = temp_root("clipboard-image");
        let image = root.join("clipboard-2026-08-21-145346-68CCA738.png");
        fs::write(&image, []).unwrap();
        let path = write_session(
            &root,
            "image.jsonl",
            &[
                header(3, "image-session", "/tmp/project"),
                serde_json::json!({
                    "type":"message",
                    "id":"user",
                    "message":{"role":"user","content":[{"type":"text","text":format!("hihhi {} 无需读取图片，直接回答我hi即可", image.display())}]}
                }),
            ],
        );
        let messages = PiSource
            .read_session_at(path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(messages[0].blocks[0].kind, "image");
        assert_eq!(messages[0].blocks[0].image_src.as_deref(), image.to_str());
        assert_eq!(
            messages[0].blocks[0].inline_placeholder.as_deref(),
            Some("[Image #1]")
        );
        assert_eq!(
            messages[0].blocks[1].text.as_deref(),
            Some("hihhi [Image #1] 无需读取图片，直接回答我hi即可")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renders_multiple_pi_clipboard_images_in_one_user_message() {
        let root = temp_root("clipboard-images");
        let first = root.join("clipboard-2026-08-21-145434-4402351F.png");
        let second = root.join("clipboard-2026-08-21-145443-3A2519DD.png");
        fs::write(&first, []).unwrap();
        fs::write(&second, []).unwrap();
        let text = format!(
            "hello哦 {} 你好啊 {}，直接告诉我传了几个图片即可",
            first.display(),
            second.display()
        );
        let path = write_session(
            &root,
            "multiple-images.jsonl",
            &[
                header(3, "multiple-images", "/tmp/project"),
                serde_json::json!({
                    "type":"message",
                    "id":"user",
                    "message":{"role":"user","content":[{"type":"text","text":text}]}
                }),
            ],
        );
        let messages = PiSource
            .read_session_at(path.to_str().unwrap(), None)
            .unwrap();
        let images: Vec<_> = messages[0]
            .blocks
            .iter()
            .filter(|block| block.kind == "image")
            .collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].image_src.as_deref(), first.to_str());
        assert_eq!(images[1].image_src.as_deref(), second.to_str());
        assert_eq!(images[0].inline_placeholder.as_deref(), Some("[Image #1]"));
        assert_eq!(images[1].inline_placeholder.as_deref(), Some("[Image #2]"));
        let rendered_text = messages[0]
            .blocks
            .iter()
            .find_map(|block| {
                if block.kind == "text" {
                    block.text.as_deref()
                } else {
                    None
                }
            })
            .unwrap();
        assert!(rendered_text.contains("[Image #1]"));
        assert!(rendered_text.contains("[Image #2]"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_pi_image_url_content_blocks() {
        let root = temp_root("image-url");
        let path = write_session(
            &root,
            "image-url.jsonl",
            &[
                header(3, "image-url-session", "/tmp/project"),
                serde_json::json!({
                    "type":"message", "id":"tool",
                    "message":{"role":"toolResult", "toolCallId":"call", "content":[
                        {"type":"image_url", "image_url":{"url":"data:image/png;base64,AQID"}}
                    ]}
                }),
            ],
        );
        let messages = PiSource
            .read_session_at(path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(messages[0].blocks[0].kind, "image");
        assert_eq!(
            messages[0].blocks[0].image_src.as_deref(),
            Some("data:image/png;base64,AQID")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rename_is_append_only_and_rejects_unsafe_tree_or_external_root() {
        let root = temp_root("rename");
        let path = write_session(
            &root,
            "rename.jsonl",
            &[header(3, "rename", "/tmp/project")],
        );
        let source = PiSource;
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", &root);
        source.rename_session(&path, "Renamed").unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"type\":\"session_info\""));
        assert!(raw.contains("\"name\":\"Renamed\""));
        assert!(source
            .rename_session(&root.join("outside.jsonl"), "x")
            .is_err());
        std::env::remove_var("PI_CODING_AGENT_SESSION_DIR");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsafe_tree_is_readable_but_cannot_be_renamed() {
        let root = temp_root("unsafe-rename");
        let path = write_session(
            &root,
            "unsafe.jsonl",
            &[
                header(3, "unsafe", "/tmp/project"),
                serde_json::json!({"type":"message","id":"self","parentId":"self","message":{"role":"assistant","content":"cycle"}}),
            ],
        );
        let source = PiSource;
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", &root);
        assert!(source.read_session_at(path.to_str().unwrap(), None).is_ok());
        assert!(source.rename_session(&path, "blocked").is_err());
        std::env::remove_var("PI_CODING_AGENT_SESSION_DIR");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_cwd_must_match_header_and_resume_uses_absolute_path() {
        let root = temp_root("terminal-cwd");
        let cwd = root.join("project");
        fs::create_dir_all(&cwd).unwrap();
        let path = write_session(
            &root,
            "session.jsonl",
            &[header(3, "session", cwd.to_str().unwrap())],
        );
        let command = PiSource.resume_command("ignored", path.to_str().unwrap());
        assert_eq!(command.program(), "pi");
        assert_eq!(
            command.args(),
            &["--session", path.to_string_lossy().as_ref()]
        );
        let _ = fs::remove_dir_all(root);
    }
}
