// opencode 会话源：~/.local/share/opencode/opencode.db —— 单个 SQLite 库，
// **不是** per-session JSONL 文件。这是与其它 agent 的本质差异。
//
// 库结构（opencode 1.17.x）：
//   project  : id(sha1) / worktree(项目目录) / vcs / time_*
//   session  : id("ses_…"，含下划线) / project_id / parent_id(子 agent 会话) / slug /
//              title / directory / model(JSON) / tokens_* 5 列 / cost(美元) /
//              time_created / time_updated / time_archived(内建归档)
//   message  : id("msg_…") / session_id / time_created / data(JSON 信封：role /
//              modelID / providerID / tokens{input,output,reasoning,cache{read,write}} /
//              cost / time{created,completed})
//   part     : id("prt_…") / message_id / session_id / time_created / data(JSON 正文：
//              type ∈ text|reasoning|tool|file|step-start|step-finish|…)
//
// 约定：
//   - `SessionMeta.path` 用虚拟路径 `opencode://<session_id>`（没有 per-session 文件）。
//     所有以 path 为入参的 trait 方法都先解析出 session id 再查库。
//   - 一律只读连接（SQLITE_OPEN_READ_ONLY + busy_timeout）—— opencode TUI 正在写
//     WAL 时并发读是安全的。**唯一写操作是 rename_session**（UPDATE session.title），
//     单独走读写连接。
//   - 子 agent 会话（parent_id 非空）不进会话列表；但它们是实打实的 API 调用，
//     统计通过 discover_stats_sessions / discover_session_companions 纳入。
//   - session.time_archived 映射到现有「显示归档」开关（include_codex_archived）。
//   - opencode 可挂任意 provider / 模型（deepseek、openrouter…），价目表推不出成本，
//     所以统计（read_turns）逐条 assistant message 取库里记录的真实 modelID + cost。
//   - 上游正在做消息存储迁移（空的 session_message 新表）——查询保持防御性，
//     解析失败跳过该行而不是拖垮整个列表。

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params, params_from_iter, Connection, OpenFlags};
use serde_json::Value;

use super::SessionSource;
use crate::agent_command::AgentCommand;
use crate::stats::shell::{extract_first_command, extract_mcp_server};
use crate::stats::types::{CallRecord, Turn};
use crate::types::{Block, Msg, ProjectInfo, SessionMeta, SessionPage, UsageSummary};
use crate::util::{
    format_iso8601_utc, home, mtime_millis, now_millis, parse_unified_diff, text_block,
    validate_rename_name,
};

pub struct OpencodeSource;

const VPATH_PREFIX: &str = "opencode://";

// ── 路径 / 连接 ──

fn data_dir() -> PathBuf {
    // opencode 走 xdg-basedir：优先 XDG_DATA_HOME，缺省 ~/.local/share（macOS 同样）。
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("opencode");
        }
    }
    home().join(".local").join("share").join("opencode")
}

fn db_path() -> PathBuf {
    data_dir().join("opencode.db")
}

pub fn is_virtual_path(path: &str) -> bool {
    path.starts_with(VPATH_PREFIX)
}

fn vpath(id: &str) -> String {
    format!("{VPATH_PREFIX}{id}")
}

fn session_id_of(path: &str) -> Result<&str, String> {
    path.strip_prefix(VPATH_PREFIX)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Not an opencode session path: {path}"))
}

fn db_err(e: rusqlite::Error) -> String {
    format!("opencode db error: {e}")
}

fn open_db() -> Result<Connection, String> {
    let p = db_path();
    if !p.is_file() {
        return Err("opencode database not found — run opencode at least once".to_string());
    }
    let conn = Connection::open_with_flags(
        &p,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Cannot open opencode database: {e}"))?;
    let _ = conn.busy_timeout(Duration::from_millis(2000));
    Ok(conn)
}

/// 读写连接 —— **仅 rename_session 使用**。不带 CREATE：库不存在直接报错。
fn open_db_rw() -> Result<Connection, String> {
    let p = db_path();
    if !p.is_file() {
        return Err("opencode database not found".to_string());
    }
    let conn = Connection::open_with_flags(
        &p,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Cannot open opencode database: {e}"))?;
    let _ = conn.busy_timeout(Duration::from_millis(2000));
    Ok(conn)
}

// ── 共享 SQL 片段 ──

/// 会话行 → SessionMeta 用的统一列清单（表别名固定为 s）。
/// message_count 只数用户消息（与 agy 的 USER_INPUT 口径一致）；
/// size 用 part 数据长度近似（没有文件可 stat）。
const SESSION_COLS: &str = "s.id, s.slug, s.title, s.directory, s.time_created, s.time_updated, s.time_archived, \
     (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id AND json_extract(m.data, '$.role') = 'user'), \
     (SELECT COALESCE(SUM(LENGTH(p2.data)), 0) FROM part p2 WHERE p2.session_id = s.id)";

fn row_to_meta(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMeta> {
    let id: String = r.get(0)?;
    let slug: String = r.get(1)?;
    let title: String = r.get(2)?;
    let dir: String = r.get(3)?;
    let created: i64 = r.get(4)?;
    let updated: i64 = r.get(5)?;
    let archived: Option<i64> = r.get(6)?;
    let msg_count: i64 = r.get(7)?;
    let size: i64 = r.get(8)?;
    Ok(SessionMeta {
        path: vpath(&id),
        file_name: id.clone(),
        id,
        title: if title.trim().is_empty() { slug } else { title },
        cwd: Some(dir),
        created: ms_to_iso(created),
        modified: updated.max(0) as u64,
        size: size.max(0) as u64,
        message_count: msg_count.max(0) as usize,
        codex_app_list_rank: None,
        codex_app_list_scanned: 0,
        codex_app_first_page_size: 0,
        codex_app_first_page_position: 0,
        codex_internal: false,
        codex_archived: archived.is_some(),
    })
}

fn ms_to_iso(ms: i64) -> Option<String> {
    if ms <= 0 {
        return None;
    }
    Some(format_iso8601_utc(ms / 1000, (ms % 1000) as u32))
}

// ── token / usage 映射 ──

/// message 信封里的 tokens{} → UsageSummary。字段缺失记 0。
fn usage_from_tokens(t: &Value) -> UsageSummary {
    let g = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
    let gc = |k: &str| {
        t.get("cache")
            .and_then(|c| c.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    UsageSummary {
        input_tokens: g("input"),
        output_tokens: g("output"),
        reasoning_output_tokens: g("reasoning"),
        cache_read_input_tokens: gc("read"),
        cache_creation_input_tokens: gc("write"),
        cache_creation_1h_input_tokens: 0,
        total: 0,
    }
    .finalize()
}

// ── opencode DB trash dump ──

const TARGET_SESSIONS_CTE: &str = "WITH RECURSIVE target(id) AS (\
    SELECT id FROM session WHERE id = ?1 \
    UNION ALL \
    SELECT s.id FROM session s JOIN target t ON s.parent_id = t.id\
)";

fn sql_value_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).to_string()),
        ValueRef::Blob(b) => Value::String(String::from_utf8_lossy(b).to_string()),
    }
}

fn json_to_sql_value(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(if *b { 1 } else { 0 }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SqlValue::Integer(i)
            } else if let Some(u) = n.as_u64().and_then(|u| i64::try_from(u).ok()) {
                SqlValue::Integer(u)
            } else {
                SqlValue::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => SqlValue::Text(s.clone()),
        Value::Array(_) | Value::Object(_) => SqlValue::Text(v.to_string()),
    }
}

fn query_json_rows(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<Vec<Value>, String> {
    let mut stmt = conn.prepare(sql).map_err(db_err)?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = stmt.query(args).map_err(db_err)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(db_err)? {
        let mut obj = serde_json::Map::new();
        for (i, name) in names.iter().enumerate() {
            obj.insert(
                name.clone(),
                sql_value_to_json(row.get_ref(i).map_err(db_err)?),
            );
        }
        out.push(Value::Object(obj));
    }
    Ok(out)
}

fn write_jsonl_line(w: &mut impl Write, v: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *w, v).map_err(|e| format!("Failed to write opencode dump: {e}"))?;
    w.write_all(b"\n")
        .map_err(|e| format!("Failed to write opencode dump: {e}"))
}

fn dump_session(conn: &Connection, sid: &str, dest: &Path) -> Result<(), String> {
    let projects = query_json_rows(
        conn,
        &format!(
            "{TARGET_SESSIONS_CTE} \
             SELECT DISTINCT p.* FROM project p \
             JOIN session s ON s.project_id = p.id \
             WHERE s.id IN (SELECT id FROM target) \
             ORDER BY p.id"
        ),
        params![sid],
    )?;
    let sessions = query_json_rows(
        conn,
        &format!(
            "{TARGET_SESSIONS_CTE} \
             SELECT s.* FROM session s \
             WHERE s.id IN (SELECT id FROM target) \
             ORDER BY CASE WHEN s.id = ?1 THEN 0 ELSE 1 END, s.time_created, s.id"
        ),
        params![sid],
    )?;
    if sessions.is_empty() {
        return Err("Session not found in opencode database".to_string());
    }
    let messages = query_json_rows(
        conn,
        &format!(
            "{TARGET_SESSIONS_CTE} \
             SELECT m.* FROM message m \
             WHERE m.session_id IN (SELECT id FROM target) \
             ORDER BY m.session_id, m.time_created, m.id"
        ),
        params![sid],
    )?;
    let parts = query_json_rows(
        conn,
        &format!(
            "{TARGET_SESSIONS_CTE} \
             SELECT p.* FROM part p \
             WHERE p.session_id IN (SELECT id FROM target) \
             ORDER BY p.session_id, p.time_created, p.id"
        ),
        params![sid],
    )?;

    let mut f =
        fs::File::create(dest).map_err(|e| format!("Failed to create opencode trash dump: {e}"))?;
    write_jsonl_line(
        &mut f,
        &serde_json::json!({
            "kind": "opencode-session",
            "version": 1,
            "session": sessions[0],
        }),
    )?;
    for row in projects {
        write_jsonl_line(
            &mut f,
            &serde_json::json!({ "kind": "opencode-project-row", "row": row }),
        )?;
    }
    for row in sessions {
        write_jsonl_line(
            &mut f,
            &serde_json::json!({ "kind": "opencode-session-row", "row": row }),
        )?;
    }
    for row in messages {
        write_jsonl_line(
            &mut f,
            &serde_json::json!({ "kind": "opencode-message-row", "row": row }),
        )?;
    }
    for row in parts {
        write_jsonl_line(
            &mut f,
            &serde_json::json!({ "kind": "opencode-part-row", "row": row }),
        )?;
    }
    Ok(())
}

fn delete_session_rows(conn: &Connection, sid: &str) -> Result<(), String> {
    conn.execute(
        &format!(
            "{TARGET_SESSIONS_CTE} \
             DELETE FROM part WHERE session_id IN (SELECT id FROM target)"
        ),
        params![sid],
    )
    .map_err(db_err)?;
    conn.execute(
        &format!(
            "{TARGET_SESSIONS_CTE} \
             DELETE FROM message WHERE session_id IN (SELECT id FROM target)"
        ),
        params![sid],
    )
    .map_err(db_err)?;
    let n = conn
        .execute(
            &format!(
                "{TARGET_SESSIONS_CTE} \
                 DELETE FROM session WHERE id IN (SELECT id FROM target)"
            ),
            params![sid],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err("Session not found in opencode database".to_string());
    }
    Ok(())
}

fn soft_delete_impl(conn: &mut Connection, sid: &str, dest: &Path) -> Result<(), String> {
    let tx = conn.transaction().map_err(db_err)?;
    dump_session(&tx, sid, dest)?;
    if let Err(e) = delete_session_rows(&tx, sid) {
        let _ = fs::remove_file(dest);
        return Err(e);
    }
    if let Err(e) = tx.commit().map_err(db_err) {
        let _ = fs::remove_file(dest);
        return Err(e);
    }
    Ok(())
}

pub fn soft_delete_to_trash(path: &str, dest: &Path) -> Result<(), String> {
    let sid = session_id_of(path)?.to_string();
    let mut conn = open_db_rw()?;
    soft_delete_impl(&mut conn, &sid, dest)
}

fn row_object(v: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    v.get("row")
        .and_then(Value::as_object)
        .ok_or_else(|| "Corrupted opencode trash dump: missing row".to_string())
}

fn insert_json_row(
    conn: &Connection,
    table: &str,
    row: &serde_json::Map<String, Value>,
    or_ignore: bool,
) -> Result<(), String> {
    if row.is_empty() {
        return Ok(());
    }
    let cols: Vec<&String> = row.keys().collect();
    let placeholders = std::iter::repeat_n("?", cols.len())
        .collect::<Vec<_>>()
        .join(", ");
    let verb = if or_ignore {
        "INSERT OR IGNORE"
    } else {
        "INSERT"
    };
    let sql = format!(
        "{verb} INTO {table} ({}) VALUES ({placeholders})",
        cols.iter()
            .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let values: Vec<SqlValue> = cols.iter().map(|c| json_to_sql_value(&row[*c])).collect();
    conn.execute(&sql, params_from_iter(values))
        .map_err(db_err)?;
    Ok(())
}

fn restore_impl(conn: &mut Connection, dump: &Path) -> Result<(), String> {
    let f = fs::File::open(dump).map_err(|e| format!("Failed to read opencode trash dump: {e}"))?;
    let mut projects = Vec::new();
    let mut sessions = Vec::new();
    let mut messages = Vec::new();
    let mut parts = Vec::new();
    for line in std::io::BufReader::new(f).lines() {
        let line = line.map_err(|e| format!("Failed to read opencode trash dump: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line)
            .map_err(|e| format!("Corrupted opencode trash dump: {e}"))?;
        match v.get("kind").and_then(Value::as_str).unwrap_or("") {
            "opencode-project-row" => projects.push(v),
            "opencode-session-row" => sessions.push(v),
            "opencode-message-row" => messages.push(v),
            "opencode-part-row" => parts.push(v),
            _ => {}
        }
    }
    if sessions.is_empty() {
        return Err("Corrupted opencode trash dump: missing session rows".to_string());
    }

    let tx = conn.transaction().map_err(db_err)?;
    for v in projects {
        insert_json_row(&tx, "project", row_object(&v)?, true)?;
    }
    for v in sessions {
        insert_json_row(&tx, "session", row_object(&v)?, false)?;
    }
    for v in messages {
        insert_json_row(&tx, "message", row_object(&v)?, false)?;
    }
    for v in parts {
        insert_json_row(&tx, "part", row_object(&v)?, false)?;
    }
    tx.commit().map_err(db_err)
}

pub fn restore_from_trash(dump: &Path) -> Result<(), String> {
    let mut conn = open_db_rw()?;
    restore_impl(&mut conn, dump)
}

// ── part → Block 映射 ──

fn file_part_image_src(p: &Value) -> Option<String> {
    let mime = p.get("mime").and_then(Value::as_str)?;
    if !mime.starts_with("image/") {
        return None;
    }
    let url = p.get("url").and_then(Value::as_str)?;
    if url.starts_with("data:") || url.starts_with("http") {
        Some(url.to_string())
    } else {
        None
    }
}

static IMAGE_REF_RE: once_cell::sync::Lazy<regex_lite::Regex> =
    once_cell::sync::Lazy::new(|| regex_lite::Regex::new(r"\[Image \d+\]\s*").unwrap());

fn part_into_blocks(p: &Value, out: &mut Vec<Block>) {
    match p.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                // opencode 在文本里内联 `[Image N]` 占位符，但图片已从 file part 渲染
                let cleaned = IMAGE_REF_RE.replace_all(t, "").trim().to_string();
                if !cleaned.is_empty() {
                    out.push(text_block("text", &cleaned));
                }
            }
        }
        "reasoning" => {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                if !t.trim().is_empty() {
                    out.push(text_block("thinking", t));
                }
            }
        }
        "tool" => {
            let tool = p
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let call_id = p.get("callID").and_then(Value::as_str).map(str::to_string);
            let state = p.get("state").cloned().unwrap_or(Value::Null);

            let input = state
                .get("input")
                .filter(|v| !v.is_null())
                .filter(|v| v.as_object().map(|o| !o.is_empty()).unwrap_or(true));
            out.push(Block {
                kind: "tool_use".into(),
                tool_name: Some(tool.clone()),
                tool_input: input.map(|v| serde_json::to_string_pretty(v).unwrap_or_default()),
                tool_id: call_id.clone(),
                ..Default::default()
            });

            // 结果块：completed → output；error → error 文本。running/pending（历史会话
            // 里的中断残留）不出结果块。edit 类工具把 unified diff 放在 metadata.diff。
            let status = state.get("status").and_then(Value::as_str).unwrap_or("");
            let file_path = state
                .pointer("/input/filePath")
                .and_then(Value::as_str)
                .map(str::to_string);
            let diff = state
                .pointer("/metadata/diff")
                .and_then(Value::as_str)
                .map(parse_unified_diff)
                .filter(|h| !h.is_empty());
            let (text, is_error) = match status {
                "completed" => (
                    state
                        .get("output")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    false,
                ),
                "error" => (
                    state
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or(Some("(error)".to_string())),
                    true,
                ),
                _ => (None, false),
            };
            let result_text = if diff.is_some() {
                None
            } else {
                text.map(|t| truncate_tool_output(&t))
            };
            let has_text = result_text
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if has_text || diff.is_some() || is_error {
                out.push(Block {
                    kind: "tool_result".into(),
                    tool_name: Some(tool),
                    tool_id: call_id,
                    text: result_text,
                    is_error,
                    file_path,
                    diff,
                    ..Default::default()
                });
            }
        }
        "file" => {
            if let Some(src) = file_part_image_src(p) {
                out.push(Block {
                    kind: "image".into(),
                    image_src: Some(src),
                    ..Default::default()
                });
            } else if let Some(name) = p.get("filename").and_then(Value::as_str) {
                out.push(Block {
                    kind: "file".into(),
                    file_path: Some(name.to_string()),
                    ..Default::default()
                });
            }
        }
        // step-start / step-finish / patch / snapshot 等对渲染无意义
        _ => {}
    }
}

const MAX_TOOL_OUTPUT_LINES: usize = 30;

fn truncate_tool_output(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_TOOL_OUTPUT_LINES {
        return text.to_string();
    }
    let mut out: Vec<&str> = lines[..MAX_TOOL_OUTPUT_LINES].to_vec();
    out.push("");
    let msg = format!("… ({} more lines)", lines.len() - MAX_TOOL_OUTPUT_LINES);
    let combined = out.join("\n");
    format!("{combined}\n{msg}")
}

// ── 库读取 ──

/// 一个会话的所有 part，按 message_id 分组、组内时间序。
fn load_parts(conn: &Connection, sid: &str) -> Result<HashMap<String, Vec<Value>>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT message_id, data FROM part WHERE session_id = ?1 ORDER BY time_created, id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![sid], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(db_err)?;
    let mut map: HashMap<String, Vec<Value>> = HashMap::new();
    for row in rows {
        let (mid, data) = row.map_err(db_err)?;
        if let Ok(v) = serde_json::from_str::<Value>(&data) {
            map.entry(mid).or_default().push(v);
        }
    }
    Ok(map)
}

/// (message_id, 信封 JSON, time_created 列) 按时间序。
fn load_messages(conn: &Connection, sid: &str) -> Result<Vec<(String, Value, i64)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, data, time_created FROM message WHERE session_id = ?1 ORDER BY time_created, id")
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![sid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (mid, data, t) = row.map_err(db_err)?;
        let env = serde_json::from_str::<Value>(&data).unwrap_or(Value::Null);
        out.push((mid, env, t));
    }
    Ok(out)
}

/// `<system-reminder>` 开头的 text part 是 opencode IDE 自动注入的系统上下文
/// （如"用户打开了某文件"），不是用户手敲的——拆成独立的系统 Msg。
fn is_injected_context_text(text: &str) -> bool {
    let t = text.trim_start();
    (t.len() > 500 && t.starts_with("# ")) || t.starts_with("Called the Read tool")
}

fn is_system_text(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<system-reminder>") || t.starts_with("<system_reminder>")
}

/// opencode 把 skill/context 文件整段注入到 user message 里：只有一个 text block，
/// 内容以 markdown 标题 `# ` 开头，且长度超过 500 字符——不是用户手打的。
fn is_injected_context(blocks: &[Block]) -> bool {
    if blocks.len() != 1 {
        return false;
    }
    let b = &blocks[0];
    if b.kind != "text" {
        return false;
    }
    let text = match &b.text {
        Some(t) => t,
        None => return false,
    };
    text.len() > 500 && text.trim_start().starts_with("# ")
}

/// opencode 的 `@file` 引用会把 Read tool 的完整输出注入到 user message 文本里：
///   Called the Read tool with the following input: {"filePath":"..."}
///   <path>...</path>
///   <type>file</type>
///   <content>
///   ... full file content ...
///   </content>
/// 或者图片版：
///   Called the Read tool with the following input: {"filePath":"..."}
///   Image read successfully
///
/// 从 text 中剥离这些注入内容，返回清理后的文本和提取到的文件路径。
fn strip_read_tool_injections(text: &str) -> (String, Vec<String>) {
    let mut paths: Vec<String> = Vec::new();
    let mut result = text.to_string();

    // 模式1: "Called the Read tool ..." → </content> 或 "Image read successfully"
    while let Some(start) = result.find("Called the Read tool with the following input:") {
        let after = &result[start..];
        if let Some(fp) = extract_json_file_path(after) {
            paths.push(fp);
        }
        let end = if let Some(ce) = after.find("</content>") {
            start + ce + "</content>".len()
        } else if let Some(ie) = after.find("Image read successfully") {
            start + ie + "Image read successfully".len()
        } else {
            start + after.find('\n').unwrap_or(after.len())
        };
        result = format!("{}{}", &result[..start], &result[end..]);
    }

    // 模式2: 裸 <path>...</path> ... <content>...</content> 块
    while let Some(ps) = result.find("<path>") {
        let after = &result[ps..];
        let pe = match after.find("</path>") {
            Some(i) => i + "</path>".len(),
            None => break,
        };
        let path = after["<path>".len()..pe - "</path>".len()]
            .trim()
            .to_string();
        if !path.is_empty() {
            paths.push(path);
        }
        let end = if let Some(ce) = after.find("</content>") {
            ps + ce + "</content>".len()
        } else {
            ps + pe
        };
        result = format!("{}{}", &result[..ps], &result[end..]);
    }

    // 清理残留文本
    while let Some(i) = result.find("[file] ") {
        let end = result[i..]
            .find('\n')
            .map(|n| i + n)
            .unwrap_or(result.len());
        result = format!("{}{}", &result[..i], &result[end..]);
    }
    while let Some(i) = result.find("Image read successfully") {
        let end = i + "Image read successfully".len();
        result = format!("{}{}", &result[..i], &result[end..]);
    }
    // 去掉 @file 引用文本
    let cleaned = strip_at_file_refs(&result);
    (cleaned.trim().to_string(), paths)
}

fn extract_json_file_path(text: &str) -> Option<String> {
    let i = text.find("\"filePath\"")?;
    let rest = &text[i..];
    let colon = rest.find(':')?;
    let q1 = rest[colon..].find('"')? + colon + 1;
    let q2 = rest[q1..].find('"')? + q1;
    Some(rest[q1..q2].to_string())
}

fn strip_at_file_refs(text: &str) -> String {
    use once_cell::sync::Lazy;
    use regex_lite::Regex;
    static RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"@\[?[A-Za-z0-9_./-]+\.[A-Za-z0-9]+\]?").unwrap());
    RE.replace_all(text, "").to_string()
}

fn read(conn: &Connection, sid: &str) -> Result<Vec<Msg>, String> {
    let mut parts = load_parts(conn, sid)?;
    let messages = load_messages(conn, sid)?;
    let mut msgs: Vec<Msg> = Vec::new();
    for (mid, env, t_col) in messages {
        let role = env
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        let ts_ms = env
            .pointer("/time/created")
            .and_then(Value::as_i64)
            .unwrap_or(t_col);
        let model = env
            .get("modelID")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut blocks: Vec<Block> = Vec::new();
        let mut system_blocks: Vec<Block> = Vec::new();
        for p in parts.remove(&mid).unwrap_or_default() {
            if role == "user" {
                if let Some(t) = p.get("type").and_then(Value::as_str) {
                    if t == "text" {
                        if let Some(text) = p.get("text").and_then(Value::as_str) {
                            if is_system_text(text) {
                                system_blocks.push(text_block("text", text));
                                continue;
                            }
                        }
                    }
                }
            }
            part_into_blocks(&p, &mut blocks);
        }
        // 系统注入 → 独立 Msg，不跟用户正文混
        if !system_blocks.is_empty() {
            msgs.push(Msg {
                uuid: None,
                role: "user".into(),
                timestamp: ms_to_iso(ts_ms),
                model: None,
                sidechain: false,
                blocks: system_blocks,
                meta_kind: Some("system".into()),
            });
        }
        // opencode 的 @file 引用会把 Read tool 的完整输出注入到 user message 文本里，
        // 剥离后提取文件路径作为 file chip（已有的不重复添加）。
        if role == "user" {
            let mut all_paths: Vec<String> = Vec::new();
            for b in &mut blocks {
                if b.kind == "text" {
                    if let Some(ref text) = b.text {
                        let (cleaned, paths) = strip_read_tool_injections(text);
                        if !paths.is_empty() || cleaned != *text {
                            b.text = if cleaned.is_empty() {
                                None
                            } else {
                                Some(cleaned)
                            };
                            all_paths.extend(paths);
                        }
                    }
                }
            }
            blocks.retain(|b| {
                !(b.kind == "text"
                    && b.text.as_deref().map(|s| s.is_empty()).unwrap_or(true)
                    && b.image_src.is_none())
            });
            let existing_images: Vec<String> = blocks
                .iter()
                .filter(|b| b.kind == "image")
                .filter_map(|b| b.image_src.clone())
                .collect();
            let existing_files: Vec<String> = blocks
                .iter()
                .filter(|b| b.kind == "file")
                .filter_map(|b| b.file_path.clone())
                .collect();
            let mut extra: Vec<Block> = Vec::new();
            for p in all_paths {
                let fname = p.rsplit('/').next().unwrap_or("");
                let is_img_path = PathBuf::from(&p)
                    .extension()
                    .map(|e| {
                        matches!(
                            e.to_str().unwrap_or(""),
                            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
                        )
                    })
                    .unwrap_or(false);
                // "file" part 已经用 data: URL 创建了图片 → 按文件名去重
                if is_img_path && existing_images.iter().any(|_| true) {
                    continue;
                }
                let file_dup = existing_files
                    .iter()
                    .chain(extra.iter().filter_map(|b| b.file_path.as_ref()))
                    .any(|f| f.ends_with(fname));
                if file_dup {
                    continue;
                }
                let pb = PathBuf::from(&p);
                if pb.exists() && !pb.is_dir() && crate::util::is_image_file(&pb) {
                    extra.push(Block {
                        kind: "image".into(),
                        image_src: Some(p),
                        ..Default::default()
                    });
                } else {
                    extra.push(Block {
                        kind: "file".into(),
                        file_path: Some(p),
                        ..Default::default()
                    });
                }
            }
            if !extra.is_empty() {
                blocks.splice(0..0, extra);
            }
        }
        if blocks.is_empty() {
            continue;
        }
        // opencode 会把 skill/context 文件整段注入到 user message 里（只有一个长
        // markdown text part，以 `# ` 开头）——标记为 context，前端按系统块渲染。
        let meta_kind = if role == "user" && is_injected_context(&blocks) {
            Some("context".into())
        } else {
            None
        };
        msgs.push(Msg {
            uuid: Some(mid),
            role,
            timestamp: ms_to_iso(ts_ms),
            model,
            sidechain: false,
            blocks,
            meta_kind,
        });
    }
    Ok(msgs)
}

fn last_user_text_sql(conn: &Connection, sid: &str) -> Option<String> {
    let mut stmt = conn
        .prepare(
            "SELECT p.data FROM part p \
         JOIN message m ON p.message_id = m.id \
         WHERE m.session_id = ?1 \
         AND json_extract(m.data, '$.role') = 'user' \
         AND json_extract(p.data, '$.type') = 'text' \
         ORDER BY m.time_created DESC \
         LIMIT 10",
        )
        .ok()?;
    let mut rows = stmt.query(rusqlite::params![sid]).ok()?;
    while let Ok(Some(row)) = rows.next() {
        let raw: String = row.get(0).ok()?;
        let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        let text = v.get("text").and_then(Value::as_str).unwrap_or("");
        if is_system_text(text) || is_injected_context_text(text) {
            continue;
        }
        let clean = crate::util::truncate_subtitle(text);
        if !clean.is_empty() {
            return Some(clean);
        }
    }
    None
}

fn read_turns_impl(conn: &Connection, sid: &str) -> Result<Vec<Turn>, String> {
    let mut parts = load_parts(conn, sid)?;
    let messages = load_messages(conn, sid)?;
    let mut turns: Vec<Turn> = Vec::new();
    let mut cur: Option<Turn> = None;

    for (mid, env, t_col) in messages {
        let role = env.get("role").and_then(Value::as_str).unwrap_or("");
        let ts_ms = env
            .pointer("/time/created")
            .and_then(Value::as_i64)
            .unwrap_or(t_col);
        let mps = parts.remove(&mid).unwrap_or_default();

        match role {
            "user" => {
                if let Some(t) = cur.take() {
                    turns.push(t);
                }
                let user_text: String = mps
                    .iter()
                    .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                cur = Some(Turn {
                    timestamp_ms: ts_ms,
                    user_message: user_text,
                    ..Default::default()
                });
            }
            "assistant" => {
                let turn = cur.get_or_insert_with(|| Turn {
                    timestamp_ms: ts_ms,
                    ..Default::default()
                });
                let mut tools: Vec<String> = Vec::new();
                let mut bash_commands: Vec<String> = Vec::new();
                let mut mcp_servers: Vec<String> = Vec::new();
                let mut has_agent_spawn = false;
                for p in &mps {
                    if p.get("type").and_then(Value::as_str) != Some("tool") {
                        continue;
                    }
                    let Some(name) = p.get("tool").and_then(Value::as_str) else {
                        continue;
                    };
                    tools.push(name.to_string());
                    if name == "task" {
                        has_agent_spawn = true;
                    }
                    if name == "bash" {
                        if let Some(cmd) = p
                            .pointer("/state/input/command")
                            .and_then(Value::as_str)
                            .and_then(extract_first_command)
                        {
                            bash_commands.push(cmd);
                        }
                    }
                    if let Some(server) = extract_mcp_server(name) {
                        mcp_servers.push(server);
                    }
                }
                let usage = env.get("tokens").map(usage_from_tokens).unwrap_or_default();
                // cost 直接用库里记录的真实值 —— opencode 可挂任意模型，价目表推不出。
                let cost_usd = env.get("cost").and_then(Value::as_f64).unwrap_or(0.0);
                let mode = env.get("mode").and_then(Value::as_str).unwrap_or("");
                let agent = env.get("agent").and_then(Value::as_str).unwrap_or("");
                turn.calls.push(CallRecord {
                    call_count: 1,
                    model: env
                        .get("modelID")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    message_id: Some(mid),
                    usage,
                    cost_usd,
                    pricing_missing: false,
                    pricing_estimated: false,
                    tools,
                    bash_commands,
                    mcp_servers,
                    has_plan_mode: mode == "plan" || agent == "plan",
                    has_agent_spawn,
                });
            }
            _ => {}
        }
    }
    if let Some(t) = cur {
        turns.push(t);
    }
    Ok(turns)
}

fn list_projects_impl(
    conn: &Connection,
    include_archived: bool,
) -> Result<Vec<ProjectInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.worktree, COUNT(s.id), COALESCE(MAX(s.time_updated), p.time_updated) \
             FROM project p \
             LEFT JOIN session s \
               ON s.project_id = p.id \
              AND s.parent_id IS NULL \
              AND (?1 OR s.time_archived IS NULL) \
             GROUP BY p.id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![include_archived], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, worktree, count, last) = row.map_err(db_err)?;
        // 没有可见会话的项目不进侧栏（归档开关切开后自然回来）
        if count == 0 {
            continue;
        }
        out.push(ProjectInfo {
            dir_name: id,
            display_path: worktree.clone(),
            session_count: count.max(0) as usize,
            last_modified: last.max(0) as u64,
            exists: Path::new(&worktree).is_dir(),
            bookmarked: false,
            parent_dir_name: None,
            worktree_name: None,
        });
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.last_modified));
    Ok(out)
}

fn list_sessions_impl(
    conn: &Connection,
    project_key: &str,
    offset: usize,
    limit: usize,
    include_archived: bool,
) -> Result<SessionPage, String> {
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session s \
             WHERE s.project_id = ?1 AND s.parent_id IS NULL AND (?2 OR s.time_archived IS NULL)",
            params![project_key, include_archived],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let limit_i = i64::try_from(limit).unwrap_or(i64::MAX);
    let offset_i = i64::try_from(offset).unwrap_or(i64::MAX);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM session s \
             WHERE s.project_id = ?1 AND s.parent_id IS NULL AND (?2 OR s.time_archived IS NULL) \
             ORDER BY s.time_updated DESC LIMIT ?3 OFFSET ?4"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map(
            params![project_key, include_archived, limit_i, offset_i],
            row_to_meta,
        )
        .map_err(db_err)?;
    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row.map_err(db_err)?);
    }
    Ok(SessionPage {
        total: total.max(0) as usize,
        sessions,
    })
}

/// 统计口径的会话发现：**不过滤** parent / 归档 —— 子 agent 会话和归档会话都是
/// 实打实的 API 花费。
fn stats_sessions_impl(conn: &Connection, project_key: &str) -> Result<Vec<SessionMeta>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM session s WHERE s.project_id = ?1 ORDER BY s.time_updated DESC"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![project_key], row_to_meta)
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(db_err)?);
    }
    Ok(out)
}

/// 单会话统计的同伴：它派生的子 agent 会话（parent_id = 该会话）。
fn companion_sessions_impl(conn: &Connection, sid: &str) -> Result<Vec<SessionMeta>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM session s WHERE s.parent_id = ?1 ORDER BY s.time_created"
        ))
        .map_err(db_err)?;
    let rows = stmt.query_map(params![sid], row_to_meta).map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(db_err)?);
    }
    Ok(out)
}

// ── SessionSource 实现 ──

impl SessionSource for OpencodeSource {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn list_projects(
        &self,
        _include_codex_internal: bool,
        include_codex_archived: bool,
    ) -> Result<Vec<ProjectInfo>, String> {
        // 没装过 / 没跑过 opencode → 空列表而非报错（与其它 agent 的空目录行为一致）
        if !db_path().is_file() {
            return Ok(Vec::new());
        }
        let conn = open_db()?;
        list_projects_impl(&conn, include_codex_archived)
    }

    fn list_sessions(
        &self,
        project_key: &str,
        offset: usize,
        limit: usize,
        _include_codex_internal: bool,
        include_codex_archived: bool,
    ) -> Result<SessionPage, String> {
        if !db_path().is_file() {
            return Ok(SessionPage {
                total: 0,
                sessions: Vec::new(),
            });
        }
        let conn = open_db()?;
        list_sessions_impl(&conn, project_key, offset, limit, include_codex_archived)
    }

    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String> {
        let sid = session_id_of(path)?;
        let conn = open_db()?;
        read(&conn, sid)
    }

    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String> {
        let trimmed = validate_rename_name(name)?;
        let sid = session_id_of(&path.to_string_lossy())?.to_string();
        // 唯一直接写 opencode 库的操作：单列 UPDATE，WAL + busy_timeout 下与 TUI 并发安全。
        let conn = open_db_rw()?;
        let n = conn
            .execute(
                "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3",
                params![trimmed, now_millis() as i64, sid],
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err("Session not found in opencode database".to_string());
        }
        Ok(())
    }

    fn validate_session_path(&self, path: &Path) -> Result<(), String> {
        if is_virtual_path(&path.to_string_lossy()) {
            Ok(())
        } else {
            Err("Not an opencode session path".to_string())
        }
    }

    fn trash_title(&self, path: &Path) -> String {
        // 回收站 dump（P2 落地）首行：{"kind":"opencode-session","session":{…session 行…}}
        let Ok(f) = fs::File::open(path) else {
            return String::new();
        };
        let mut first = String::new();
        if std::io::BufReader::new(f).read_line(&mut first).is_err() {
            return String::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(&first) else {
            return String::new();
        };
        v.pointer("/session/title")
            .or_else(|| v.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn resume_command(&self, session_id: &str, _path: &str) -> AgentCommand {
        AgentCommand::new("opencode")
            .arg("--session")
            .arg(session_id)
    }

    fn new_session_command(&self) -> AgentCommand {
        AgentCommand::new("opencode")
    }

    fn image_src(&self, block: &Value) -> Option<String> {
        file_part_image_src(block)
    }

    fn usage_summary(&self, path: &str) -> Result<UsageSummary, String> {
        let sid = session_id_of(path)?;
        let conn = open_db()?;
        let u = conn
            .query_row(
                "SELECT tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write \
                 FROM session WHERE id = ?1",
                params![sid],
                |r| {
                    Ok(UsageSummary {
                        input_tokens: r.get::<_, i64>(0)?.max(0) as u64,
                        output_tokens: r.get::<_, i64>(1)?.max(0) as u64,
                        reasoning_output_tokens: r.get::<_, i64>(2)?.max(0) as u64,
                        cache_read_input_tokens: r.get::<_, i64>(3)?.max(0) as u64,
                        cache_creation_input_tokens: r.get::<_, i64>(4)?.max(0) as u64,
                        cache_creation_1h_input_tokens: 0,
                        total: 0,
                    })
                },
            )
            .map_err(db_err)?;
        Ok(u.finalize())
    }

    fn context_usage(&self, path: &str) -> Result<UsageSummary, String> {
        let sid = session_id_of(path)?;
        let conn = open_db()?;
        // 末尾第一条带非零 usage 的 assistant message ≈ 当前上下文规模
        let mut stmt = conn
            .prepare(
                "SELECT data FROM message WHERE session_id = ?1 ORDER BY time_created DESC, id DESC LIMIT 50",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![sid], |r| r.get::<_, String>(0))
            .map_err(db_err)?;
        for row in rows {
            let Ok(data) = row else { continue };
            let Ok(env) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            if env.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            if let Some(t) = env.get("tokens") {
                let u = usage_from_tokens(t);
                if u.total > 0 {
                    return Ok(u);
                }
            }
        }
        Ok(UsageSummary::default())
    }

    fn last_prompt(&self, path: &str) -> Result<Option<String>, String> {
        let sid = session_id_of(path)?;
        let conn = open_db()?;
        Ok(last_user_text_sql(&conn, sid))
    }

    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String> {
        let sid = session_id_of(path)?;
        let conn = open_db()?;
        read_turns_impl(&conn, sid)
    }

    fn discover_stats_sessions(&self, project_key: &str) -> Result<Vec<SessionMeta>, String> {
        if !db_path().is_file() {
            return Ok(Vec::new());
        }
        let conn = open_db()?;
        stats_sessions_impl(&conn, project_key)
    }

    fn discover_session_companions(&self, path: &str) -> Vec<SessionMeta> {
        let Ok(sid) = session_id_of(path) else {
            return Vec::new();
        };
        let Ok(conn) = open_db() else {
            return Vec::new();
        };
        companion_sessions_impl(&conn, sid).unwrap_or_default()
    }

    fn source_mtime(&self, _path: &str) -> u64 {
        // 库文件（含 WAL）的 mtime 作为所有会话的失效锚点：粗粒度但绝不漏失效，
        // 而 opencode 的单会话重读只是一次索引查询，过度失效的代价可忽略。
        let db = db_path();
        let wal = db.with_extension("db-wal");
        mtime_millis(&db).max(mtime_millis(&wal))
    }

    fn contains_text(&self, path: &str, q_lower: &str) -> bool {
        let Ok(sid) = session_id_of(path) else {
            return false;
        };
        let Ok(conn) = open_db() else {
            return false;
        };
        // instr(lower(x), q)：ASCII 大小写折叠与 file_contains_ci 对齐；CJK 无大小写直接匹配。
        // 只是预筛 —— 命中后仍由 find_text_hit 在「用户消息 text 块」里做精确匹配。
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM part WHERE session_id = ?1 AND instr(lower(data), ?2) > 0)",
            params![sid, q_lower],
            |r| r.get::<_, bool>(0),
        )
        .unwrap_or(false)
    }

    fn watch_target(&self, _path: &str) -> Option<PathBuf> {
        // 实时 tail 盯库文件：任何会话写入都会动 WAL；fingerprint 短路让无关写入
        // 只花一次 stat。WAL 被 checkpoint 清掉时退回主库文件。
        let db = db_path();
        let wal = db.with_extension("db-wal");
        Some(if wal.exists() { wal } else { db })
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存库 + 最小 schema（只建查询用到的列）。
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE project (
                id TEXT PRIMARY KEY, worktree TEXT NOT NULL, time_updated INTEGER NOT NULL
             );
             CREATE TABLE session (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT,
                slug TEXT NOT NULL DEFAULT '', title TEXT NOT NULL DEFAULT '',
                directory TEXT NOT NULL DEFAULT '',
                tokens_input INTEGER NOT NULL DEFAULT 0, tokens_output INTEGER NOT NULL DEFAULT 0,
                tokens_reasoning INTEGER NOT NULL DEFAULT 0, tokens_cache_read INTEGER NOT NULL DEFAULT 0,
                tokens_cache_write INTEGER NOT NULL DEFAULT 0, cost REAL NOT NULL DEFAULT 0,
                time_created INTEGER NOT NULL DEFAULT 0, time_updated INTEGER NOT NULL DEFAULT 0,
                time_archived INTEGER
             );
             CREATE TABLE message (
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL DEFAULT 0, data TEXT NOT NULL
             );
             CREATE TABLE part (
                id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL DEFAULT 0, data TEXT NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    fn insert_msg(conn: &Connection, sid: &str, mid: &str, t: i64, data: &Value) {
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            params![mid, sid, t, data.to_string()],
        )
        .unwrap();
    }

    fn insert_part(conn: &Connection, sid: &str, mid: &str, pid: &str, t: i64, data: &Value) {
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pid, mid, sid, t, data.to_string()],
        )
        .unwrap();
    }

    fn seed_basic_session(conn: &Connection) {
        conn.execute(
            "INSERT INTO project (id, worktree, time_updated) VALUES ('proj1', '/tmp/demo', 100)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, project_id, slug, title, directory, time_created, time_updated,
                tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write)
             VALUES ('ses_a', 'proj1', 'lucky-moon', 'Greeting', '/tmp/demo', 1000, 2000,
                100, 20, 5, 30, 10)",
            [],
        )
        .unwrap();
        // user: hello
        insert_msg(
            conn,
            "ses_a",
            "msg_1",
            1000,
            &serde_json::json!({"role":"user","time":{"created":1000}}),
        );
        insert_part(
            conn,
            "ses_a",
            "msg_1",
            "prt_1",
            1000,
            &serde_json::json!({"type":"text","text":"hello opencode"}),
        );
        // assistant: reasoning + text + tool
        insert_msg(
            conn,
            "ses_a",
            "msg_2",
            1001,
            &serde_json::json!({
                "role":"assistant","modelID":"deepseek-v4-pro","providerID":"deepseek",
                "cost":0.005,"mode":"build",
                "tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":30,"write":10}},
                "time":{"created":1001,"completed":1500}
            }),
        );
        insert_part(
            conn,
            "ses_a",
            "msg_2",
            "prt_2",
            1001,
            &serde_json::json!({"type":"reasoning","text":"let me think"}),
        );
        insert_part(
            conn,
            "ses_a",
            "msg_2",
            "prt_3",
            1002,
            &serde_json::json!({"type":"text","text":"Hi!"}),
        );
        insert_part(
            conn,
            "ses_a",
            "msg_2",
            "prt_4",
            1003,
            &serde_json::json!({
                "type":"tool","tool":"bash","callID":"call_1",
                "state":{"status":"completed","input":{"command":"git status"},"output":"clean"}
            }),
        );
    }

    #[test]
    fn read_maps_parts_to_blocks_in_order() {
        let conn = test_db();
        seed_basic_session(&conn);
        let msgs = read(&conn, "ses_a").unwrap();
        assert_eq!(msgs.len(), 2);

        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].uuid.as_deref(), Some("msg_1"));
        assert_eq!(msgs[0].blocks[0].kind, "text");
        assert_eq!(msgs[0].blocks[0].text.as_deref(), Some("hello opencode"));

        let a = &msgs[1];
        assert_eq!(a.role, "assistant");
        assert_eq!(a.model.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(a.blocks.len(), 4); // thinking + text + tool_use + tool_result
        assert_eq!(a.blocks[0].kind, "thinking");
        assert_eq!(a.blocks[1].kind, "text");
        assert_eq!(a.blocks[2].kind, "tool_use");
        assert_eq!(a.blocks[2].tool_name.as_deref(), Some("bash"));
        assert!(a.blocks[2]
            .tool_input
            .as_deref()
            .unwrap()
            .contains("git status"));
        assert_eq!(a.blocks[3].kind, "tool_result");
        assert_eq!(a.blocks[3].text.as_deref(), Some("clean"));
    }

    #[test]
    fn read_skips_empty_envelope_messages() {
        let conn = test_db();
        seed_basic_session(&conn);
        // 只有信封、没有 part 的进行中消息
        insert_msg(
            &conn,
            "ses_a",
            "msg_3",
            1004,
            &serde_json::json!({"role":"assistant","modelID":"x"}),
        );
        let msgs = read(&conn, "ses_a").unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn tool_error_state_maps_to_error_result() {
        let conn = test_db();
        seed_basic_session(&conn);
        insert_msg(
            &conn,
            "ses_a",
            "msg_4",
            1005,
            &serde_json::json!({"role":"assistant","modelID":"m"}),
        );
        insert_part(
            &conn,
            "ses_a",
            "msg_4",
            "prt_9",
            1005,
            &serde_json::json!({
                "type":"tool","tool":"bash","callID":"c9",
                "state":{"status":"error","input":{"command":"boom"},"error":"exit 1"}
            }),
        );
        let msgs = read(&conn, "ses_a").unwrap();
        let last = msgs.last().unwrap();
        let result = last
            .blocks
            .iter()
            .find(|b| b.kind == "tool_result")
            .unwrap();
        assert!(result.is_error);
        assert_eq!(result.text.as_deref(), Some("exit 1"));
    }

    #[test]
    fn edit_tool_metadata_diff_becomes_structured_diff() {
        let conn = test_db();
        seed_basic_session(&conn);
        insert_msg(
            &conn,
            "ses_a",
            "msg_5",
            1006,
            &serde_json::json!({"role":"assistant","modelID":"m"}),
        );
        insert_part(
            &conn,
            "ses_a",
            "msg_5",
            "prt_10",
            1006,
            &serde_json::json!({
                "type":"tool","tool":"edit","callID":"c10",
                "state":{
                    "status":"completed",
                    "input":{"filePath":"/tmp/foo.ts"},
                    "output":"",
                    "metadata":{"diff":"@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n"}
                }
            }),
        );
        let msgs = read(&conn, "ses_a").unwrap();
        let last = msgs.last().unwrap();
        let result = last
            .blocks
            .iter()
            .find(|b| b.kind == "tool_result")
            .unwrap();
        assert_eq!(result.file_path.as_deref(), Some("/tmp/foo.ts"));
        let hunks = result.diff.as_ref().expect("diff parsed");
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l.kind == "add"));
    }

    #[test]
    fn image_file_part_becomes_image_block() {
        let conn = test_db();
        seed_basic_session(&conn);
        insert_msg(
            &conn,
            "ses_a",
            "msg_6",
            1007,
            &serde_json::json!({"role":"user"}),
        );
        insert_part(
            &conn,
            "ses_a",
            "msg_6",
            "prt_11",
            1007,
            &serde_json::json!({"type":"file","mime":"image/png","filename":"a.png","url":"data:image/png;base64,AAAA"}),
        );
        insert_part(
            &conn,
            "ses_a",
            "msg_6",
            "prt_12",
            1008,
            &serde_json::json!({"type":"text","text":"look at this"}),
        );
        let msgs = read(&conn, "ses_a").unwrap();
        let last = msgs.last().unwrap();
        assert_eq!(last.blocks[0].kind, "image");
        assert!(last.blocks[0]
            .image_src
            .as_deref()
            .unwrap()
            .starts_with("data:image/png"));
        assert_eq!(last.blocks[1].kind, "text");
    }

    #[test]
    fn list_projects_counts_visible_sessions_only() {
        let conn = test_db();
        seed_basic_session(&conn);
        // 子 agent 会话 + 归档会话都不该计入默认列表
        conn.execute(
            "INSERT INTO session (id, project_id, parent_id, slug, directory, time_created, time_updated)
             VALUES ('ses_child', 'proj1', 'ses_a', 'child', '/tmp/demo', 1500, 1500)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, project_id, slug, directory, time_created, time_updated, time_archived)
             VALUES ('ses_arch', 'proj1', 'old', '/tmp/demo', 1600, 1600, 1700)",
            [],
        )
        .unwrap();
        let projects = list_projects_impl(&conn, false).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].session_count, 1);
        assert_eq!(projects[0].display_path, "/tmp/demo");

        let with_archived = list_projects_impl(&conn, true).unwrap();
        assert_eq!(with_archived[0].session_count, 2);
    }

    #[test]
    fn list_sessions_paginates_and_maps_meta() {
        let conn = test_db();
        seed_basic_session(&conn);
        let page = list_sessions_impl(&conn, "proj1", 0, 10, false).unwrap();
        assert_eq!(page.total, 1);
        let s = &page.sessions[0];
        assert_eq!(s.id, "ses_a");
        assert_eq!(s.path, "opencode://ses_a");
        assert_eq!(s.title, "Greeting");
        assert_eq!(s.cwd.as_deref(), Some("/tmp/demo"));
        assert_eq!(s.message_count, 1); // 只数用户消息
        assert!(s.size > 0);
        assert_eq!(s.modified, 2000);
    }

    #[test]
    fn list_sessions_falls_back_to_slug_when_title_empty() {
        let conn = test_db();
        seed_basic_session(&conn);
        conn.execute("UPDATE session SET title = '' WHERE id = 'ses_a'", [])
            .unwrap();
        let page = list_sessions_impl(&conn, "proj1", 0, 10, false).unwrap();
        assert_eq!(page.sessions[0].title, "lucky-moon");
    }

    #[test]
    fn soft_delete_dumps_rows_and_restore_reinserts_them() {
        let mut conn = test_db();
        seed_basic_session(&conn);
        let dump = std::env::temp_dir().join(format!(
            "opencode-trash-test-{}-{}.jsonl",
            std::process::id(),
            now_millis()
        ));

        soft_delete_impl(&mut conn, "ses_a", &dump).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM session WHERE id = 'ses_a'", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM message WHERE session_id = 'ses_a'",
                [],
                |r| { r.get::<_, i64>(0) }
            )
            .unwrap(),
            0
        );
        assert_eq!(OpencodeSource.trash_title(&dump), "Greeting");

        restore_impl(&mut conn, &dump).unwrap();
        let page = list_sessions_impl(&conn, "proj1", 0, 10, false).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.sessions[0].id, "ses_a");
        assert_eq!(read(&conn, "ses_a").unwrap().len(), 2);

        let _ = fs::remove_file(dump);
    }

    #[test]
    fn read_turns_fills_model_cost_and_tools_per_call() {
        let conn = test_db();
        seed_basic_session(&conn);
        let turns = read_turns_impl(&conn, "ses_a").unwrap();
        assert_eq!(turns.len(), 1);
        let t = &turns[0];
        assert_eq!(t.user_message, "hello opencode");
        assert_eq!(t.calls.len(), 1);
        let c = &t.calls[0];
        assert_eq!(c.model, "deepseek-v4-pro");
        assert_eq!(c.message_id.as_deref(), Some("msg_2"));
        assert!((c.cost_usd - 0.005).abs() < 1e-9);
        assert_eq!(c.usage.input_tokens, 100);
        assert_eq!(c.usage.cache_read_input_tokens, 30);
        assert_eq!(c.usage.total, 165); // 100+20+5+30+10
        assert_eq!(c.tools, vec!["bash".to_string()]);
        assert_eq!(c.bash_commands, vec!["git".to_string()]);
    }

    #[test]
    fn read_turns_multiple_models_in_one_session() {
        let conn = test_db();
        seed_basic_session(&conn);
        // 同一会话里换了模型再来一轮 —— opencode 的多模型场景
        insert_msg(
            &conn,
            "ses_a",
            "msg_7",
            2000,
            &serde_json::json!({"role":"user","time":{"created":2000}}),
        );
        insert_part(
            &conn,
            "ses_a",
            "msg_7",
            "prt_20",
            2000,
            &serde_json::json!({"type":"text","text":"try another model"}),
        );
        insert_msg(
            &conn,
            "ses_a",
            "msg_8",
            2001,
            &serde_json::json!({
                "role":"assistant","modelID":"claude-sonnet-5","providerID":"anthropic",
                "cost":0.12,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}}
            }),
        );
        insert_part(
            &conn,
            "ses_a",
            "msg_8",
            "prt_21",
            2001,
            &serde_json::json!({"type":"text","text":"ok"}),
        );
        let turns = read_turns_impl(&conn, "ses_a").unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].calls[0].model, "deepseek-v4-pro");
        assert_eq!(turns[1].calls[0].model, "claude-sonnet-5");
        assert!((turns[1].calls[0].cost_usd - 0.12).abs() < 1e-9);
    }

    #[test]
    fn session_id_of_parses_virtual_path() {
        assert_eq!(session_id_of("opencode://ses_abc").unwrap(), "ses_abc");
        assert!(session_id_of("/real/file.jsonl").is_err());
        assert!(session_id_of("opencode://").is_err());
    }

    #[test]
    fn usage_from_tokens_handles_missing_fields() {
        let u = usage_from_tokens(&serde_json::json!({"input": 7}));
        assert_eq!(u.input_tokens, 7);
        assert_eq!(u.cache_read_input_tokens, 0);
        assert_eq!(u.total, 7);
    }

    #[test]
    fn file_part_image_src_rejects_non_image() {
        assert!(file_part_image_src(&serde_json::json!({
            "type":"file","mime":"application/pdf","url":"data:application/pdf;base64,AA"
        }))
        .is_none());
        assert_eq!(
            file_part_image_src(&serde_json::json!({
                "type":"file","mime":"image/jpeg","url":"data:image/jpeg;base64,BB"
            })),
            Some("data:image/jpeg;base64,BB".to_string())
        );
    }
}
