// Antigravity CLI (agy) 会话源：~/.gemini/antigravity-cli/brain/<uuid>/
//
// 数据布局：
//   ~/.gemini/antigravity-cli/
//   ├── brain/<conversation-uuid>/
//   │   ├── .system_generated/logs/
//   │   │   ├── transcript.jsonl           ← 滚动窗口（CHECKPOINT 压缩）
//   │   │   └── transcript_full.jsonl      ← 名义上"完整"，但也会被 CHECKPOINT 截断；选更大的那个
//   │   └── media__<ts>.<ext>              ← 用户上传的附件
//   └── history.jsonl                      ← {display, timestamp, workspace, conversationId}
//
// step 行结构：
//   {step_index, source(USER_EXPLICIT|MODEL|SYSTEM), type, status, created_at, content,
//    thinking?, tool_calls?[{name, args}], truncated_fields?}
//
// 关键坑：
//   - USER_INPUT 的 content 带 XML 壳：<USER_REQUEST>正文</USER_REQUEST> + metadata
//   - 工具结果 content 有 "Created At: …\nCompleted At: …\n" 两行前缀
//   - CODE_ACTION 里含 [diff_block_start] + unified diff
//   - transcript.jsonl 不是 append-only —— CLI 随时可能整文件重写（CHECKPOINT 压缩），
//     watcher 需处理 file-shrink
//   - 无 token/usage/model 字段 → 统计零占位

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::SessionSource;
use crate::agent_command::AgentCommand;
use crate::stats::types::{CallRecord, Turn};
use crate::types::{Block, DiffHunk, Msg, ProjectInfo, SessionMeta, SessionPage, UsageSummary};
use crate::util::{
    append_jsonl_line, clean_title, home, mtime_millis, parse_iso8601_ms, parse_unified_diff,
    text_block, validate_rename_name,
};

pub struct AgySource;

// ── 路径帮助 ──

fn cli_data_dir() -> PathBuf {
    home().join(".gemini").join("antigravity-cli")
}

fn ide_data_dir() -> PathBuf {
    home().join(".gemini").join("antigravity")
}

fn history_path() -> PathBuf {
    cli_data_dir().join("history.jsonl")
}

fn is_uuid_dir(name: &str) -> bool {
    // 标准 UUID：8-4-4-4-12，全小写 hex + dash
    name.len() == 36 && name.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// 返回所有 brain 目录（CLI + IDE）中合法 UUID 会话及其 transcript 路径。
/// `from_ide` = true 表示来自 IDE（不支持 CLI resume）。
/// 同一 UUID 若同时出现在 CLI 和 IDE，CLI 优先（先入 seen set）。
fn all_conversations() -> Vec<(String, PathBuf, bool)> {
    let cli_brain = cli_data_dir().join("brain");
    let ide_brain = ide_data_dir().join("brain");
    let sources: Vec<(PathBuf, bool)> = vec![(cli_brain, false), (ide_brain, true)];
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (bd, from_ide) in sources {
        if !bd.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&bd) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !is_uuid_dir(&name) || !seen.insert(name.clone()) {
                continue;
            }
            let transcript = e
                .path()
                .join(".system_generated")
                .join("logs")
                .join("transcript.jsonl");
            if transcript.exists() {
                out.push((name, transcript, from_ide));
            }
        }
    }
    out
}

/// 给定一个 transcript.jsonl 路径，返回内容更完整的那个文件：
/// agy 的 CHECKPOINT 机制会重写 transcript_full.jsonl（截断早期历史），
/// 导致 transcript.jsonl 有时反而保留了更多消息。选文件更大的那个。
fn preferred_transcript(transcript_path: &Path) -> PathBuf {
    let full = transcript_path.with_file_name("transcript_full.jsonl");
    if !full.exists() {
        return transcript_path.to_path_buf();
    }
    let size_of = |p: &Path| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    if size_of(&full) >= size_of(transcript_path) {
        full
    } else {
        transcript_path.to_path_buf()
    }
}

// ── history.jsonl → workspace 映射 ──

fn load_workspace_map() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let hp = history_path();
    let Ok(file) = fs::File::open(&hp) else {
        return map;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let (Some(conv_id), Some(ws)) = (
            v.get("conversationId").and_then(Value::as_str),
            v.get("workspace").and_then(Value::as_str),
        ) {
            if !ws.is_empty() {
                map.insert(conv_id.to_string(), ws.to_string());
            }
        }
    }
    map
}

/// 从 transcript 的工具调用里推断 workspace（兜底路径）。
/// 跳过 agy 自身的内部目录（`~/.gemini/`），只取用户项目路径。
fn infer_workspace_from_transcript(path: &Path) -> Option<String> {
    let gemini_prefix = home().join(".gemini");
    let is_user_dir = |p: &str| -> bool {
        let pb = Path::new(p);
        pb.is_absolute() && !pb.starts_with(&gemini_prefix) && pb.is_dir()
    };

    let file = fs::File::open(path).ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok).take(30) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                if let Some(args) = call.get("args").and_then(Value::as_object) {
                    for key in &["DirectoryPath", "AbsolutePath", "SearchPath"] {
                        if let Some(p) = args.get(*key).and_then(Value::as_str) {
                            let p = p.trim_matches('"');
                            if is_user_dir(p) {
                                return Some(p.to_string());
                            }
                        }
                    }
                }
            }
        }
        if v.get("type").and_then(Value::as_str) == Some("VIEW_FILE") {
            if let Some(content) = v.get("content").and_then(Value::as_str) {
                if let Some(pos) = content.find("file:///") {
                    let rest = &content[pos + 7..];
                    let end = rest
                        .find('`')
                        .or_else(|| rest.find('\n'))
                        .unwrap_or(rest.len());
                    let file_path = &rest[..end];
                    if let Some(parent) = Path::new(file_path).parent() {
                        let dir = parent.to_string_lossy().to_string();
                        if is_user_dir(&dir) {
                            return Some(dir);
                        }
                    }
                }
            }
        }
    }
    None
}

// ── USER_INPUT 剥壳 ──

fn extract_user_request(content: &str) -> String {
    if let Some(start) = content.find("<USER_REQUEST>") {
        let after = &content[start + 14..];
        if let Some(end) = after.find("</USER_REQUEST>") {
            return after[..end].trim().to_string();
        }
    }
    // 兜底：没有 XML 壳就原样（一般不会）
    content.trim().to_string()
}

// ── truncated_fields 修复 ──

/// agy 的 CHECKPOINT 压缩会在 content 中插入 `<truncated N bytes>` 标记，
/// 截断可能打断 markdown 代码块围栏（如 ` ```typescript` 被拆成 `<truncated>\npescript`）。
/// 这里做两件事：
///   1. 把 `<truncated ...>` 替换为可折叠的 `[…]` 占位
///   2. 如果紧跟的行看起来是代码围栏语言标识的残片（如 `pescript`），尝试恢复为完整围栏
fn repair_truncated_content(text: &str) -> String {
    use regex_lite::Regex;
    let re = Regex::new(r"<truncated \d+ (?:bytes|lines)>").unwrap();
    if !re.is_match(text) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if re.is_match(line) {
            // Check if next line is a partial code fence language suffix
            if let Some(&next) = lines.peek() {
                let next_trimmed = next.trim();
                if is_partial_lang(next_trimmed) {
                    let lang = reconstruct_lang(next_trimmed);
                    out.push_str("\n```");
                    out.push_str(&lang);
                    out.push('\n');
                    lines.next(); // consume the partial line
                    continue;
                }
            }
            out.push_str("[…]");
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    // trim trailing extra newline
    if out.ends_with('\n') && !text.ends_with('\n') {
        out.pop();
    }
    out
}

fn is_partial_lang(s: &str) -> bool {
    let known_suffixes = [
        "pescript", "ript", "script", // typescript/javascript
        "ython", "thon", // python
        "ust",  // rust
        "tml", "html", // html
        "css", "son", "json", // css/json
        "ell", "shell", // shell
        "ash", "bash", // bash
        "aml", "yaml", // yaml
        "oml", "toml", // toml
        "vue",  // vue
        "sx", "tsx", "jsx", // tsx/jsx
    ];
    if s.is_empty() || s.len() > 12 || s.contains(' ') {
        return false;
    }
    known_suffixes.contains(&s)
}

fn reconstruct_lang(partial: &str) -> String {
    let map: &[(&str, &str)] = &[
        ("pescript", "typescript"),
        ("ript", "typescript"),
        ("script", "typescript"),
        ("ython", "python"),
        ("thon", "python"),
        ("ust", "rust"),
        ("tml", "html"),
        ("html", "html"),
        ("css", "css"),
        ("son", "json"),
        ("json", "json"),
        ("ell", "shell"),
        ("shell", "shell"),
        ("ash", "bash"),
        ("bash", "bash"),
        ("aml", "yaml"),
        ("yaml", "yaml"),
        ("oml", "toml"),
        ("toml", "toml"),
        ("vue", "vue"),
        ("sx", "tsx"),
        ("tsx", "tsx"),
        ("jsx", "jsx"),
    ];
    for (suffix, full) in map {
        if partial == *suffix {
            return full.to_string();
        }
    }
    partial.to_string()
}

// ── 工具结果 content 剥头 ──

fn strip_tool_header(content: &str) -> &str {
    // 跳过 "Created At: ...\nCompleted At: ...\n" 开头
    let mut rest = content;
    for prefix in &["Created At:", "Completed At:"] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            if let Some(newline) = stripped.find('\n') {
                rest = &stripped[newline + 1..];
            }
        }
    }
    rest
}

/// 从 CODE_ACTION content 里提取目标文件路径。
fn extract_code_action_file(content: &str) -> Option<String> {
    // "The following changes were made by the ... tool to: /path/to/file.ts. If relevant"
    let marker = " tool to: ";
    if let Some(pos) = content.find(marker) {
        let after = &content[pos + marker.len()..];
        // 路径以句末 ". " 或换行结尾（不能单用 '.' 否则吃掉文件扩展名）
        let end = after
            .find(". ")
            .or_else(|| after.find(".\n"))
            .or_else(|| after.find('\n'))
            .unwrap_or(after.len());
        let path = after[..end].trim().trim_end_matches('.');
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    None
}

// ── diff 解析 ──

fn parse_diff_from_code_action(content: &str) -> Option<(String, Vec<DiffHunk>)> {
    let marker = "[diff_block_start]";
    let pos = content.find(marker)?;
    let diff_text = &content[pos + marker.len()..];
    let file_path = extract_code_action_file(content);
    let hunks = parse_unified_diff(diff_text);
    if hunks.is_empty() {
        return None;
    }
    Some((file_path.unwrap_or_default(), hunks))
}

// ── 标题提取 ──

const RENAME_MARKER: &str = "$rename:";

fn extract_title(path: &Path) -> String {
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    let mut title = String::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // $rename marker（末行优先）
        if let Some(rest) = line.strip_prefix(RENAME_MARKER) {
            title = rest.trim().to_string();
            continue;
        }
        // 首条 USER_REQUEST 做 fallback 标题
        if title.is_empty() {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("type").and_then(Value::as_str) == Some("USER_INPUT") {
                    if let Some(content) = v.get("content").and_then(Value::as_str) {
                        let req = extract_user_request(content);
                        if !req.is_empty() {
                            title = clean_title(&req);
                        }
                    }
                }
            }
        }
    }
    title
}

/// 轻量扫描：只计算 USER_INPUT 行数作为消息数。
fn count_user_messages(path: &Path) -> usize {
    let Ok(file) = fs::File::open(path) else {
        return 0;
    };
    let mut count = 0;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.contains("\"USER_INPUT\"") {
            count += 1;
        }
    }
    count
}

// ── read_session ──

fn extract_model_change(content: &str) -> Option<String> {
    let marker = "`Model Selection` from ";
    let pos = content.find(marker)?;
    let after = &content[pos + marker.len()..];
    let to_pos = after.find(" to ")?;
    let rest = &after[to_pos + 4..];
    let end = rest.find(". No need").or_else(|| rest.find(". "))?;
    let model = rest[..end].trim();
    if model.is_empty() || model == "None" {
        None
    } else {
        Some(model.to_string())
    }
}

/// Scan text for all `Model Selection` occurrences and return the last one.
/// Checkpoints embed prior USER_INPUTs as JSON strings, so the marker appears
/// multiple times when the user changed models before the truncation point.
fn extract_last_model(text: &str) -> Option<String> {
    let marker = "`Model Selection` from ";
    let mut last: Option<String> = None;
    let mut haystack = text;
    while let Some(pos) = haystack.find(marker) {
        let after = &haystack[pos + marker.len()..];
        if let Some(to_pos) = after.find(" to ") {
            let rest = &after[to_pos + 4..];
            let end = rest
                .find(". No need")
                .or_else(|| rest.find(". "))
                .or_else(|| rest.find("\\n"))
                .unwrap_or(rest.len());
            let model = rest[..end].trim().trim_end_matches('.');
            if !model.is_empty() && model != "None" {
                last = Some(model.to_string());
            }
        }
        haystack = &haystack[pos + marker.len()..];
    }
    last
}

/// agy 的 CHECKPOINT 截断 transcript，早期消息丢失。
/// 但每个会话目录有 .git，历史 snapshot 里保留了完整 transcript。
/// 二分 git log 找到最后一个 transcript_full.jsonl 从 step 0 开始的 commit，
/// 取出 step_index < current_first_step 的行，拼在当前文件前面。
fn recover_early_lines(transcript_dir: &Path, first_step: u64) -> Vec<String> {
    if first_step == 0 {
        return Vec::new();
    }
    // transcript_dir = .../logs/ (parent of the .jsonl file)
    let conv_dir = transcript_dir
        .parent() // .system_generated/
        .and_then(|p| p.parent()); // <uuid>/
    let Some(conv_dir) = conv_dir else {
        return Vec::new();
    };
    if !conv_dir.join(".git").is_dir() {
        return Vec::new();
    }
    let log_output = crate::util::silent_command("git")
        .args(["log", "--format=%H", "--reverse"])
        .current_dir(conv_dir)
        .output();
    let Ok(log_output) = log_output else {
        return Vec::new();
    };
    let commits: Vec<&str> = std::str::from_utf8(&log_output.stdout)
        .unwrap_or("")
        .lines()
        .collect();
    if commits.is_empty() {
        return Vec::new();
    }

    let rel_full = ".system_generated/logs/transcript_full.jsonl";

    let starts_at_zero = |commit: &str| -> bool {
        let show = crate::util::silent_command("git")
            .args(["show", &format!("{commit}:{rel_full}")])
            .current_dir(conv_dir)
            .output();
        let Ok(show) = show else { return false };
        if !show.status.success() {
            return false;
        }
        let stdout = std::str::from_utf8(&show.stdout).unwrap_or("");
        stdout
            .lines()
            .next()
            .and_then(|line| serde_json::from_str::<Value>(line).ok())
            .and_then(|v| v.get("step_index").and_then(Value::as_u64))
            == Some(0)
    };

    // 二分：commits 按时间升序（--reverse），找最后一个 starts_at_zero 的
    // 单调性：前面的 commit 从 step 0 开始，某个 commit 后 CHECKPOINT 截断变成非 0
    let mut lo = 0usize;
    let mut hi = commits.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if starts_at_zero(commits[mid]) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    // lo-1 是最后一个 starts_at_zero 的 commit（如果存在）
    if lo == 0 {
        return Vec::new();
    }
    let best = commits[lo - 1];

    let show = crate::util::silent_command("git")
        .args(["show", &format!("{best}:{rel_full}")])
        .current_dir(conv_dir)
        .output();
    let Ok(show) = show else {
        return Vec::new();
    };
    let stdout = std::str::from_utf8(&show.stdout).unwrap_or("");
    stdout
        .lines()
        .filter(|line| {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                v.get("step_index")
                    .and_then(Value::as_u64)
                    .is_some_and(|s| s < first_step)
            } else {
                false
            }
        })
        .map(|s| s.to_string())
        .collect()
}

fn read(path: &str) -> Result<Vec<Msg>, String> {
    let transcript_path = Path::new(path);
    let read_path = preferred_transcript(transcript_path);
    let file = fs::File::open(&read_path)
        .map_err(|e| format!("Cannot open {}: {e}", read_path.display()))?;

    // 检查第一行的 step_index，如果 > 0 说明有被截断的早期消息
    let all_lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    let first_step = all_lines
        .first()
        .and_then(|line| serde_json::from_str::<Value>(line).ok())
        .and_then(|v| v.get("step_index").and_then(Value::as_u64))
        .unwrap_or(0);
    let early_lines = recover_early_lines(read_path.parent().unwrap_or(Path::new("")), first_step);

    let mut msgs: Vec<Msg> = Vec::new();
    let mut current_model = String::new();

    for line in early_lines.iter().chain(all_lines.iter()) {
        if line.starts_with(RENAME_MARKER) {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let step_type = v.get("type").and_then(Value::as_str).unwrap_or("");
        let source = v.get("source").and_then(Value::as_str).unwrap_or("");
        let created_at = v
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_string);

        match step_type {
            // ── 用户消息 ──
            "USER_INPUT" if source == "USER_EXPLICIT" => {
                let content = v.get("content").and_then(Value::as_str).unwrap_or("");
                if let Some(model) = extract_model_change(content) {
                    current_model = model;
                }
                let text = extract_user_request(content);
                if text.is_empty() {
                    continue;
                }
                msgs.push(Msg {
                    role: "user".into(),
                    timestamp: created_at,
                    blocks: vec![text_block("text", &text)],
                    ..Default::default()
                });
            }

            // ── 助手回复 ──
            "PLANNER_RESPONSE" if source == "MODEL" => {
                let mut blocks: Vec<Block> = Vec::new();

                // thinking
                if let Some(thinking) = v.get("thinking").and_then(Value::as_str) {
                    if !thinking.trim().is_empty() {
                        blocks.push(text_block("thinking", thinking));
                    }
                }

                // content（正文）— 修复 truncated_fields 截断导致的 markdown 断裂
                if let Some(content) = v.get("content").and_then(Value::as_str) {
                    if !content.trim().is_empty() {
                        let repaired = repair_truncated_content(content);
                        blocks.push(text_block("text", &repaired));
                    }
                }

                // tool_calls → tool_use 块
                if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let name = call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        let args = call.get("args").cloned().unwrap_or(Value::Null);
                        let input = if args.is_null()
                            || args.as_object().map(|m| m.is_empty()).unwrap_or(false)
                        {
                            None
                        } else {
                            Some(serde_json::to_string_pretty(&args).unwrap_or_default())
                        };
                        blocks.push(Block {
                            kind: "tool_use".into(),
                            tool_name: Some(name.to_string()),
                            tool_input: input,
                            ..Default::default()
                        });
                    }
                }

                if !blocks.is_empty() {
                    msgs.push(Msg {
                        role: "assistant".into(),
                        timestamp: created_at,
                        model: if current_model.is_empty() {
                            None
                        } else {
                            Some(current_model.clone())
                        },
                        blocks,
                        ..Default::default()
                    });
                }
            }

            // ── 工具结果 ──
            "VIEW_FILE" | "LIST_DIRECTORY" | "GREP_SEARCH" | "RUN_COMMAND" | "SEARCH_WEB"
            | "ASK_QUESTION" => {
                let raw = v.get("content").and_then(Value::as_str).unwrap_or("");
                let body = strip_tool_header(raw);
                msgs.push(Msg {
                    role: "user".into(),
                    timestamp: created_at,
                    blocks: vec![Block {
                        kind: "tool_result".into(),
                        tool_name: Some(step_type.to_string()),
                        text: Some(body.to_string()),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
            }

            // ── 代码编辑（diff） ──
            "CODE_ACTION" if source == "MODEL" => {
                let raw = v.get("content").and_then(Value::as_str).unwrap_or("");
                let body = strip_tool_header(raw);
                if let Some((file_path, hunks)) = parse_diff_from_code_action(body) {
                    msgs.push(Msg {
                        role: "user".into(),
                        timestamp: created_at,
                        blocks: vec![Block {
                            kind: "tool_result".into(),
                            tool_name: Some("Edit".into()),
                            text: None,
                            file_path: if file_path.is_empty() {
                                None
                            } else {
                                Some(file_path)
                            },
                            diff: Some(hunks),
                            ..Default::default()
                        }],
                        ..Default::default()
                    });
                } else {
                    // diff 解析失败 → 纯文本兜底
                    msgs.push(Msg {
                        role: "user".into(),
                        timestamp: created_at,
                        blocks: vec![Block {
                            kind: "tool_result".into(),
                            tool_name: Some("CODE_ACTION".into()),
                            text: Some(body.to_string()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    });
                }
            }

            // ── 系统事件 → 折叠 system 卡片 ──
            "CHECKPOINT" | "SYSTEM_MESSAGE" => {
                let content = v.get("content").and_then(Value::as_str).unwrap_or("");
                // Checkpoint embeds prior USER_INPUTs as JSON; extract the last model set before truncation.
                if step_type == "CHECKPOINT" && current_model.is_empty() {
                    if let Some(model) = extract_last_model(content) {
                        current_model = model;
                    }
                }
                if !content.trim().is_empty() {
                    msgs.push(Msg {
                        role: "user".into(),
                        timestamp: created_at,
                        meta_kind: Some("system".into()),
                        blocks: vec![text_block("text", content)],
                        ..Default::default()
                    });
                }
            }

            // CONVERSATION_HISTORY 一般空 content → 跳过
            _ => {}
        }
    }

    Ok(msgs)
}

// ── read_turns（统计用） ──

fn last_user_text(fp: &Path) -> Option<String> {
    let raw = fs::read(fp).ok()?;
    for line in raw.rsplit(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("USER_INPUT") {
            continue;
        }
        let content = v.get("content").and_then(Value::as_str)?;
        // 提取 <USER_REQUEST>...</USER_REQUEST> 中的文本
        let text = if let Some(start) = content.find("<USER_REQUEST>") {
            let after = &content[start + "<USER_REQUEST>".len()..];
            if let Some(end) = after.find("</USER_REQUEST>") {
                after[..end].trim()
            } else {
                after.trim()
            }
        } else {
            content.trim()
        };
        let clean = crate::util::truncate_subtitle(text);
        if !clean.is_empty() {
            return Some(clean);
        }
    }
    None
}

fn read_turns(fp: &Path) -> Vec<Turn> {
    let read_path = preferred_transcript(fp);
    let Ok(file) = fs::File::open(&read_path) else {
        return Vec::new();
    };

    let mut turns: Vec<Turn> = Vec::new();
    let mut current_turn: Option<Turn> = None;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.starts_with(RENAME_MARKER) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let step_type = v.get("type").and_then(Value::as_str).unwrap_or("");
        let source = v.get("source").and_then(Value::as_str).unwrap_or("");
        let ts_ms = v
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(parse_iso8601_ms)
            .unwrap_or(0);

        match (source, step_type) {
            ("USER_EXPLICIT", "USER_INPUT") => {
                if let Some(t) = current_turn.take() {
                    turns.push(t);
                }
                let user_text = v
                    .get("content")
                    .and_then(Value::as_str)
                    .map(extract_user_request)
                    .unwrap_or_default();
                current_turn = Some(Turn {
                    timestamp_ms: ts_ms,
                    user_message: user_text,
                    calls: Vec::new(),
                    ..Default::default()
                });
            }
            ("MODEL", "PLANNER_RESPONSE") => {
                let turn = current_turn.get_or_insert_with(|| Turn {
                    timestamp_ms: ts_ms,
                    ..Default::default()
                });
                // 只有最终结果（有 content/thinking/tool_calls）才计一次 call
                let has_substance = v
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty())
                    || v.get("thinking").is_some()
                    || v.get("tool_calls").is_some();
                if has_substance {
                    let mut tools: Vec<String> = Vec::new();
                    if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
                        for c in calls {
                            if let Some(name) = c.get("name").and_then(Value::as_str) {
                                tools.push(name.to_string());
                            }
                        }
                    }
                    turn.calls.push(CallRecord {
                        call_count: 1,
                        model: String::new(),
                        message_id: None,
                        usage: UsageSummary::default(),
                        cost_usd: 0.0,
                        pricing_missing: false,
                        pricing_estimated: false,
                        cost_source: crate::stats::types::CostSource::Unknown,
                        tools,
                        bash_commands: Vec::new(),
                        mcp_servers: Vec::new(),
                        has_plan_mode: false,
                        has_agent_spawn: false,
                    });
                }
            }
            ("MODEL", "RUN_COMMAND") => {
                // 尝试从 content 里提取命令首词
                if let Some(turn) = current_turn.as_mut() {
                    if let Some(last_call) = turn.calls.last_mut() {
                        if let Some(content) = v.get("content").and_then(Value::as_str) {
                            if let Some(cmd) = extract_bash_command(content) {
                                last_call.bash_commands.push(cmd);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(t) = current_turn {
        turns.push(t);
    }
    turns
}

fn extract_bash_command(content: &str) -> Option<String> {
    // RUN_COMMAND content 格式: "... Output:\n<actual output>"
    // 命令本身在 tool_calls 的 args 里，但结果 step 没直接带。
    // 这里就不提取了，返回 None —— bash_commands 维度对 agy 暂空。
    let _ = content;
    None
}

// ── SessionSource 实现 ──

impl SessionSource for AgySource {
    fn name(&self) -> &'static str {
        "agy"
    }

    fn list_projects(
        &self,
        _include_codex_internal: bool,
        _include_codex_archived: bool,
    ) -> Result<Vec<ProjectInfo>, String> {
        let ws_map = load_workspace_map();
        let mut map: HashMap<String, (usize, u64)> = HashMap::new();

        let home_str = home().to_string_lossy().to_string();
        for (conv_id, transcript_path, _from_ide) in all_conversations() {
            let mut workspace = ws_map
                .get(&conv_id)
                .cloned()
                .or_else(|| {
                    infer_workspace_from_transcript(&preferred_transcript(&transcript_path))
                })
                .unwrap_or_else(|| "outside-of-project".to_string());
            if workspace == home_str {
                workspace = "outside-of-project".to_string();
            }
            let mt = mtime_millis(&transcript_path);
            let entry = map.entry(workspace).or_insert((0, 0));
            entry.0 += 1;
            if mt > entry.1 {
                entry.1 = mt;
            }
        }

        let mut out: Vec<ProjectInfo> = map
            .into_iter()
            .map(|(workspace, (count, last))| {
                let is_sentinel = workspace == "outside-of-project";
                let exists = is_sentinel || Path::new(&workspace).is_dir();
                let display = if is_sentinel {
                    home().to_string_lossy().to_string()
                } else {
                    workspace.clone()
                };
                ProjectInfo {
                    dir_name: workspace,
                    display_path: display,
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
        _include_codex_internal: bool,
        _include_codex_archived: bool,
    ) -> Result<SessionPage, String> {
        let ws_map = load_workspace_map();
        let home_str = home().to_string_lossy().to_string();
        let mut matched: Vec<(String, PathBuf, u64, bool)> = Vec::new();

        for (conv_id, transcript_path, from_ide) in all_conversations() {
            let mut workspace = ws_map
                .get(&conv_id)
                .cloned()
                .or_else(|| {
                    infer_workspace_from_transcript(&preferred_transcript(&transcript_path))
                })
                .unwrap_or_else(|| "outside-of-project".to_string());
            if workspace == home_str {
                workspace = "outside-of-project".to_string();
            }
            if workspace == project_key {
                let mt = mtime_millis(&transcript_path);
                matched.push((conv_id, transcript_path, mt, from_ide));
            }
        }
        matched.sort_by_key(|m| std::cmp::Reverse(m.2));
        let total = matched.len();

        let sessions: Vec<SessionMeta> = matched
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(conv_id, tp, _, from_ide)| {
                let read_path = preferred_transcript(tp);
                let title = extract_title(&read_path);
                let message_count = count_user_messages(&read_path);
                let mt = mtime_millis(tp);
                let size = fs::metadata(tp).map(|m| m.len()).unwrap_or(0);
                let cwd = if *from_ide {
                    Some("ide://antigravity-chat".to_string())
                } else if project_key == "outside-of-project" {
                    Some(home().to_string_lossy().to_string())
                } else {
                    Some(project_key.to_string())
                };
                SessionMeta {
                    id: conv_id.clone(),
                    file_name: tp
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    path: tp.to_string_lossy().to_string(),
                    title,
                    cwd,
                    created: None,
                    modified: mt,
                    size,
                    message_count,
                    pi_branch_count: None,
                    pi_entry_count: None,
                    codex_app_list_rank: None,
                    codex_app_list_scanned: 0,
                    codex_app_first_page_size: 0,
                    codex_app_first_page_position: 0,
                    codex_internal: false,
                    codex_archived: false,
                }
            })
            .collect();

        Ok(SessionPage { total, sessions })
    }

    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String> {
        let mut msgs = read(path)?;
        crate::util::post_process_session_msgs(&mut msgs);
        Ok(msgs)
    }

    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String> {
        let trimmed = validate_rename_name(name)?;
        let marker_line = format!("{RENAME_MARKER}{trimmed}");
        append_jsonl_line(path, &marker_line)?;
        let full = path.with_file_name("transcript_full.jsonl");
        if full.exists() {
            append_jsonl_line(&full, &marker_line)?;
        }
        Ok(())
    }

    fn trash_title(&self, path: &Path) -> String {
        let read_path = preferred_transcript(path);
        extract_title(&read_path)
    }

    fn watch_target(&self, path: &str) -> Option<PathBuf> {
        // CHECKPOINT 会整文件重写 transcript_full；tail 盯内容更全的那个。
        Some(preferred_transcript(Path::new(path)))
    }

    fn resume_command(&self, session_id: &str, _path: &str) -> AgentCommand {
        AgentCommand::new("agy")
            .arg("--conversation")
            .arg(session_id)
    }

    fn new_session_command(&self) -> AgentCommand {
        AgentCommand::new("agy")
    }

    fn image_src(&self, _block: &Value) -> Option<String> {
        None
    }

    fn usage_summary(&self, _path: &str) -> Result<UsageSummary, String> {
        Ok(UsageSummary::default())
    }

    fn last_prompt(&self, path: &str) -> Result<Option<String>, String> {
        Ok(last_user_text(Path::new(path)))
    }

    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String> {
        let fp = Path::new(path);
        if !fp.exists() {
            return Err(format!("File not found: {path}"));
        }
        Ok(read_turns(fp))
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::parse_hunk_header;

    #[test]
    fn extract_user_request_strips_xml_wrapper() {
        let content = "<USER_REQUEST>\n你好世界\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is: 2026-07-05T09:22:52+08:00.\n</ADDITIONAL_METADATA>";
        assert_eq!(extract_user_request(content), "你好世界");
    }

    #[test]
    fn extract_user_request_handles_multiline_request() {
        let content = "<USER_REQUEST>\nfirst line\nsecond line\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\ntime\n</ADDITIONAL_METADATA>";
        assert_eq!(extract_user_request(content), "first line\nsecond line");
    }

    #[test]
    fn extract_user_request_no_wrapper_returns_trimmed() {
        assert_eq!(extract_user_request("  plain text  "), "plain text");
    }

    #[test]
    fn strip_tool_header_removes_created_completed() {
        let content = "Created At: 2026-07-05T09:22:55+08:00\nCompleted At: 2026-07-05T09:22:55+08:00\n{\"name\":\".DS_Store\"}";
        assert_eq!(strip_tool_header(content), "{\"name\":\".DS_Store\"}");
    }

    #[test]
    fn strip_tool_header_no_header_returns_as_is() {
        assert_eq!(strip_tool_header("just text"), "just text");
    }

    #[test]
    fn parse_hunk_header_basic() {
        assert_eq!(parse_hunk_header("@@ -46,7 +46,7 @@"), Some((46, 46)));
        assert_eq!(parse_hunk_header("@@ -1 +1,3 @@"), Some((1, 1)));
    }

    #[test]
    fn parse_unified_diff_produces_hunks() {
        let diff = "\n@@ -46,3 +46,3 @@\n   'pricing.family.claude': 'Claude',\n-  'pricing.family.gemini': 'Gemini',\n+  'pricing.family.gemini': 'agy',\n   'pricing.empty': '暂无价格数据。',\n";
        let hunks = parse_unified_diff(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 46);
        assert_eq!(hunks[0].new_start, 46);
        assert!(hunks[0].lines.iter().any(|l| l.kind == "del"));
        assert!(hunks[0].lines.iter().any(|l| l.kind == "add"));
    }

    #[test]
    fn parse_unified_diff_multi_hunk() {
        let diff = "@@ -1,3 +1,3 @@\n a\n-b\n+c\n@@ -10,2 +10,2 @@\n x\n-y\n+z\n";
        let hunks = parse_unified_diff(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[1].old_start, 10);
    }

    #[test]
    fn extract_code_action_file_path() {
        let content = "The following changes were made by the multi_replace_file_content tool to: /Users/wuchao/apps/project/src/foo.ts. If relevant";
        assert_eq!(
            extract_code_action_file(content),
            Some("/Users/wuchao/apps/project/src/foo.ts".to_string())
        );
    }

    #[test]
    fn is_uuid_dir_validates_standard_uuids() {
        assert!(is_uuid_dir("3d837dc5-23a3-4789-a8c1-0e75ec52b486"));
        assert!(is_uuid_dir("7c35d38f-016c-421e-bf7a-8ba14bc33d32"));
        assert!(!is_uuid_dir("tempmediaStorage"));
        assert!(!is_uuid_dir(""));
        assert!(!is_uuid_dir("not-a-uuid"));
    }

    #[test]
    fn read_parses_user_input_assistant_and_tool_result() {
        let dir = std::env::temp_dir().join("agy-test-read");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let lines = [
            r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-07-05T01:22:52Z","content":"<USER_REQUEST>\nhi\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\ntime\n</ADDITIONAL_METADATA>"}"#,
            r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-07-05T01:22:53Z","thinking":"Let me check.","content":"Hello!","tool_calls":[{"name":"list_dir","args":{"DirectoryPath":"/tmp"}}]}"#,
            r#"{"step_index":2,"source":"MODEL","type":"LIST_DIRECTORY","status":"DONE","created_at":"2026-07-05T01:22:54Z","content":"Created At: 2026-07-05T09:22:54+08:00\nCompleted At: 2026-07-05T09:22:54+08:00\nfile1\nfile2"}"#,
        ];
        fs::write(&path, lines.join("\n")).unwrap();

        let msgs = read(path.to_str().unwrap()).unwrap();
        assert_eq!(msgs.len(), 3);

        // user message
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].blocks[0].text.as_deref(), Some("hi"));

        // assistant
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].blocks.len(), 3); // thinking + text + tool_use
        assert_eq!(msgs[1].blocks[0].kind, "thinking");
        assert_eq!(msgs[1].blocks[1].kind, "text");
        assert_eq!(msgs[1].blocks[1].text.as_deref(), Some("Hello!"));
        assert_eq!(msgs[1].blocks[2].kind, "tool_use");
        assert_eq!(msgs[1].blocks[2].tool_name.as_deref(), Some("list_dir"));

        // tool result
        assert_eq!(msgs[2].role, "user");
        assert_eq!(msgs[2].blocks[0].kind, "tool_result");
        assert_eq!(msgs[2].blocks[0].text.as_deref(), Some("file1\nfile2"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_parses_code_action_into_diff_blocks() {
        let dir = std::env::temp_dir().join("agy-test-diff");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let content = r#"The following changes were made by the multi_replace_file_content tool to: /tmp/foo.ts. If relevant
[diff_block_start]
@@ -1,3 +1,3 @@
 line1
-old
+new
 line3"#;
        let line = serde_json::json!({
            "step_index": 0,
            "source": "MODEL",
            "type": "CODE_ACTION",
            "status": "DONE",
            "created_at": "2026-07-05T01:00:00Z",
            "content": content,
        });
        fs::write(&path, serde_json::to_string(&line).unwrap()).unwrap();

        let msgs = read(path.to_str().unwrap()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].blocks[0].kind, "tool_result");
        assert_eq!(msgs[0].blocks[0].tool_name.as_deref(), Some("Edit"));
        assert_eq!(msgs[0].blocks[0].file_path.as_deref(), Some("/tmp/foo.ts"));
        assert!(msgs[0].blocks[0].diff.is_some());
        let hunks = msgs[0].blocks[0].diff.as_ref().unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), 4); // ctx + del + add + ctx

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_last_model_from_checkpoint_content() {
        // Checkpoint embeds prior USER_INPUTs as JSON with escaped quotes
        let content = r#"some summary text
{"step_index":0,"content":"<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Claude Opus 4.6 (Thinking). No need to comment on this change.\n</USER_SETTINGS_CHANGE>"}
more text
{"step_index":14,"content":"<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from Claude Opus 4.6 (Thinking) to Gemini 3.5 Flash (High). No need to comment on this change.\n</USER_SETTINGS_CHANGE>"}"#;
        assert_eq!(
            extract_last_model(content),
            Some("Gemini 3.5 Flash (High)".to_string())
        );
    }

    #[test]
    fn extract_last_model_single_occurrence() {
        let content = "The user changed setting `Model Selection` from None to Claude Opus 4.6. No need to comment.";
        assert_eq!(
            extract_last_model(content),
            Some("Claude Opus 4.6".to_string())
        );
    }

    #[test]
    fn extract_last_model_none_returns_none() {
        let content = "The user changed setting `Model Selection` from Gemini 3.5 to None. No need to comment.";
        assert_eq!(extract_last_model(content), None);
    }

    #[test]
    fn extract_last_model_no_match() {
        assert_eq!(extract_last_model("no model info here"), None);
    }

    #[test]
    fn read_extracts_model_from_checkpoint() {
        let dir = std::env::temp_dir().join("agy-test-checkpoint-model");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let checkpoint_content = r#"{{ CHECKPOINT 0 }}
{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Claude Opus 4.6 (Thinking). No need to comment on this change.\n</USER_SETTINGS_CHANGE>"}"#;
        let lines = [
            serde_json::to_string(&serde_json::json!({
                "step_index": 12,
                "source": "SYSTEM",
                "type": "CHECKPOINT",
                "status": "DONE",
                "created_at": "2026-07-05T01:00:00Z",
                "content": checkpoint_content,
            }))
            .unwrap(),
            serde_json::to_string(&serde_json::json!({
                "step_index": 13,
                "source": "USER_EXPLICIT",
                "type": "USER_INPUT",
                "status": "DONE",
                "created_at": "2026-07-05T01:01:00Z",
                "content": "<USER_REQUEST>\nhello\n</USER_REQUEST>",
            }))
            .unwrap(),
            serde_json::to_string(&serde_json::json!({
                "step_index": 14,
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "status": "DONE",
                "created_at": "2026-07-05T01:01:01Z",
                "content": "Hi there!",
            }))
            .unwrap(),
        ];
        fs::write(&path, lines.join("\n")).unwrap();

        let msgs = read(path.to_str().unwrap()).unwrap();
        // checkpoint (system) + user + assistant = 3
        assert_eq!(msgs.len(), 3);
        // Assistant should have model from checkpoint
        assert_eq!(msgs[2].model.as_deref(), Some("Claude Opus 4.6 (Thinking)"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repair_truncated_restores_code_fence() {
        let input = "table row\n<truncated 115 bytes>\npescript\n// code\nimport foo\n```\nafter";
        let out = repair_truncated_content(input);
        assert!(
            out.contains("```typescript\n// code"),
            "should restore code fence, got: {out}"
        );
        assert!(
            !out.contains("\npescript\n"),
            "partial lang line should be consumed"
        );
    }

    #[test]
    fn repair_truncated_no_partial_lang() {
        let input = "before\n<truncated 100 bytes>\nsome normal text\nafter";
        let out = repair_truncated_content(input);
        assert!(out.contains("[…]"), "should replace marker with ellipsis");
        assert!(out.contains("some normal text"), "non-lang text preserved");
    }

    #[test]
    fn repair_truncated_no_markers() {
        let input = "plain text\nno truncation";
        assert_eq!(repair_truncated_content(input), input);
    }
}
