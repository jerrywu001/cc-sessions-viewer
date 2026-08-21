//! Kimi Code local-session source.
//!
//! Kimi stores one user-visible session in a directory. `state.json` is the
//! session metadata and `agents/main/wire.jsonl` is the primary transcript.
//! The parser deliberately remains minimal in this first integration phase;
//! tool/content reconstruction and usage accounting are added separately.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};

use chrono::{TimeZone, Utc};
use rayon::prelude::*;
use serde_json::Value;

use super::{SessionSource, SessionStorageKind, SessionStorageUnit};
use crate::agent_command::AgentCommand;
use crate::stats::types::Turn;
use crate::types::{Block, Msg, ProjectInfo, SessionMeta, SessionPage, UsageSummary};
use crate::util::{clean_title, home, mtime_millis, text_block, truncate_subtitle};

pub struct KimiSource;

const SESSIONS_DIR: &str = "sessions";
const STATE_FILE: &str = "state.json";
const MAIN_WIRE_RELATIVE: &str = "agents/main/wire.jsonl";
const MAX_TOOL_INPUT_BYTES: usize = 64 * 1024;
const MAX_TOOL_RESULT_BYTES: usize = 128 * 1024;
const MAX_QUESTION_TEXT_BYTES: usize = 8 * 1024;
const MAX_OPTION_LABEL_BYTES: usize = 1024;
const MAX_OPTION_DESCRIPTION_BYTES: usize = 8 * 1024;

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

pub fn kimi_home() -> PathBuf {
    let configured = std::env::var_os("KIMI_CODE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_kimi_home(configured, &home(), &current_dir)
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
    let index = sessions_dir(root).join("session_index.jsonl");
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
    let file = fs::File::open(path)
        .map_err(|error| format!("Failed to open Kimi Code session: {error}"))?;
    let mut prompts = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
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

fn read_main_wire(path: &Path) -> Result<Vec<Msg>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("Failed to open Kimi Code session: {error}"))?;
    let events: Vec<Value> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
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
            let text = event.get("input").map(prompt_text).unwrap_or_default();
            if text.is_empty() {
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
                blocks: vec![text_block("text", &text)],
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

impl SessionSource for KimiSource {
    fn name(&self) -> &'static str {
        "kimi"
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
        read_main_wire(Path::new(path))
    }

    fn rename_session(&self, _path: &Path, _name: &str) -> Result<(), String> {
        Err("Kimi Code session rename is not implemented yet".to_string())
    }

    fn trash_title(&self, path: &Path) -> String {
        let session_dir = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .unwrap_or(path);
        record_from_session_dir(&kimi_home(), session_dir)
            .map(|record| record.title)
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

    fn usage_summary(&self, _path: &str) -> Result<UsageSummary, String> {
        Ok(UsageSummary::default())
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
        if !Path::new(path).is_file() {
            return Err(format!("Kimi Code main wire does not exist: {path}"));
        }
        Ok(Vec::new())
    }

    fn source_mtime(&self, path: &str) -> u64 {
        Path::new(path)
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(session_files_mtime)
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
        let index = sessions_dir(&root).join("session_index.jsonl");
        let valid = root
            .join(SESSIONS_DIR)
            .join("wd_project_123")
            .join("session_abc");
        fs::write(
            index,
            format!(
                "{}\n{}\n",
                serde_json::json!({"sessionDir": valid}).to_string(),
                serde_json::json!({"sessionDir": "/tmp/not-kimi-session"}).to_string(),
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
}
