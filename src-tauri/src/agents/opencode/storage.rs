//! Read-only normalization of OpenCode's legacy and event-sourced session layouts.
//! Upstream reference: anomalyco/opencode@dbcd5ce9485f3338375d1fe9d92864c91e263030,
//! packages/core/src/session/{sql,history,stored}.ts. No migrations or persistent views.
use super::db_err;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub(super) fn has_table(conn: &Connection, name: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type = 'table' AND name = ?1)",
        params![name],
        |r| r.get(0),
    )
    .map_err(db_err)
}

/// TEMP views belong only to this connection, including on SQLITE_OPEN_READ_ONLY.
/// Both layouts may coexist after migration; the v2 row wins for a duplicate session ID.
/// Never shadow main tables: the legacy trash writer must still address original rows.
pub(super) fn prepare_read_views(conn: &Connection) -> Result<(), String> {
    let legacy = has_table(conn, "session")?;
    let v2 = has_table(conn, "session_v2")?;
    if !legacy && !v2 {
        return Err("Unsupported opencode schema: expected session or session_v2 table".into());
    }
    let columns = "s.id, s.project_id, s.parent_id, s.slug, COALESCE(s.title, '') AS title, \
        s.directory, s.time_created, s.time_updated, s.time_archived, \
        s.tokens_input, s.tokens_output, s.tokens_reasoning, s.tokens_cache_read, s.tokens_cache_write";
    let mut sessions = Vec::new();
    let mut messages = Vec::new();
    let mut parts = Vec::new();
    if legacy {
        sessions.push(format!(
            "SELECT {columns}, 1 AS storage FROM main.session s {}",
            if v2 {
                "WHERE NOT EXISTS (SELECT 1 FROM main.session_v2 v WHERE v.id = s.id)"
            } else {
                ""
            }
        ));
        messages.push("SELECT m.id, m.session_id, m.time_created, m.data, \
            json_extract(CASE WHEN json_valid(m.data) THEN m.data ELSE '{}' END, '$.role') AS role, \
            json_extract(CASE WHEN json_valid(m.data) THEN m.data ELSE '{}' END, '$.role') AS kind, \
            m.time_created AS sort_order, 1 AS storage \
            FROM main.message m JOIN sv_session s ON s.id = m.session_id AND s.storage = 1".to_string());
        parts.push(
            "SELECT p.message_id, p.session_id, p.data, p.time_created AS sort_order, \
            p.id AS sort_id, 1 AS storage FROM main.part p \
            JOIN sv_session s ON s.id = p.session_id AND s.storage = 1"
                .to_string(),
        );
    }
    if v2 {
        sessions.push(format!(
            "SELECT {columns}, 2 AS storage FROM main.session_v2 s"
        ));
        messages.push(
            "SELECT m.id, m.session_id, m.time_created, m.data, \
            CASE m.type WHEN 'compaction' THEN 'assistant' \
              WHEN 'synthetic' THEN 'system' WHEN 'skill' THEN 'system' \
              WHEN 'system' THEN 'system' WHEN 'user' THEN 'user' \
              WHEN 'assistant' THEN 'assistant' ELSE '' END AS role, m.type AS kind, \
            m.seq AS sort_order, 2 AS storage FROM main.session_message m"
                .to_string(),
        );
        let data = "CASE WHEN json_valid(m.data) THEN m.data ELSE '{}' END";
        parts.push(format!(
            "SELECT m.id AS message_id, m.session_id, \
             json_object('type', 'text', 'text', json_extract({data}, \
               CASE WHEN m.type = 'compaction' THEN '$.summary' ELSE '$.text' END)) AS data, \
             -1 AS sort_order, '' AS sort_id, 2 AS storage FROM main.session_message m \
             WHERE m.type IN ('user', 'system', 'synthetic', 'skill', 'compaction')"
        ));
        parts.push(format!(
            "SELECT m.id AS message_id, m.session_id, j.value AS data, \
             CAST(j.key AS INTEGER) AS sort_order, '' AS sort_id, 2 AS storage \
             FROM main.session_message m, json_each({data}, '$.files') j WHERE m.type = 'user' AND j.type = 'object'"
        ));
        let split = has_table(conn, "session_part")?;
        parts.push(format!(
            "SELECT m.id AS message_id, m.session_id, j.value AS data, \
             CAST(j.key AS INTEGER) AS sort_order, '' AS sort_id, 2 AS storage \
             FROM main.session_message m, json_each({data}, '$.content') j \
             WHERE m.type = 'assistant' AND j.type = 'object' {}",
            if split { "AND NOT EXISTS (SELECT 1 FROM main.session_part p WHERE p.message_id = m.id AND p.session_id = m.session_id)" } else { "" }
        ));
        if split {
            parts.push(
                "SELECT p.message_id, p.session_id, p.data, p.ordinal AS sort_order, \
                '' AS sort_id, 2 AS storage FROM main.session_part p \
                JOIN main.session_message m ON m.id = p.message_id AND m.session_id = p.session_id \
                WHERE m.type = 'assistant'"
                    .into(),
            );
        }
    }
    conn.execute_batch(&format!(
        "DROP VIEW IF EXISTS temp.sv_part; DROP VIEW IF EXISTS temp.sv_message; \
         DROP VIEW IF EXISTS temp.sv_session; \
         CREATE TEMP VIEW sv_session AS {}; \
         CREATE TEMP VIEW sv_message AS {}; \
         CREATE TEMP VIEW sv_part AS {};",
        sessions.join(" UNION ALL "),
        messages.join(" UNION ALL "),
        parts.join(" UNION ALL ")
    ))
    .map_err(db_err)
}

pub(super) fn normalize_envelope(mut env: Value, role: &str, storage: i64) -> Option<Value> {
    if !matches!(role, "user" | "assistant" | "system") {
        return None;
    }
    let model = env.pointer("/model/id").cloned();
    let obj = env.as_object_mut()?;
    obj.insert("role".into(), Value::from(role));
    if storage == 2 {
        if let Some(model) = model {
            obj.insert("modelID".into(), model);
        }
    }
    Some(env)
}

fn attachment(conn: &Connection, raw: &Value) -> Value {
    let mime = raw.get("mime").and_then(Value::as_str).unwrap_or("");
    let uri = raw.get("uri").and_then(Value::as_str).unwrap_or("");
    let mut url = raw
        .get("data")
        .and_then(Value::as_str)
        .map(|data| format!("data:{mime};base64,{data}"));
    if let Some(hash) = uri.strip_prefix("media:") {
        if hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
            && mime.starts_with("image/")
        {
            // A media: ref is a DB blob key, never a filesystem path or a server URL.
            let blob = conn
                .query_row(
                    "SELECT mime, data FROM main.blob WHERE hash = ?1",
                    params![hash],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
                )
                .optional();
            if let Ok(Some((mime, bytes))) = blob {
                if mime.starts_with("image/") {
                    url = Some(format!(
                        "data:{mime};base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ));
                }
            }
        }
    } else if uri.starts_with("data:") || uri.starts_with("https://") || uri.starts_with("http://")
    {
        url = Some(uri.to_string());
    }
    json!({"type":"file", "mime":mime, "url":url,
        "filename":raw.get("name").and_then(Value::as_str).unwrap_or(if uri.is_empty() { "attachment" } else { uri })})
}

/// Map v2 content to the existing block/usage contract; v1 is passed through unchanged.
pub(super) fn normalize_part(conn: &Connection, mut part: Value, storage: i64) -> Vec<Value> {
    if storage != 2 {
        return vec![part];
    }
    match part.get("type").and_then(Value::as_str) {
        Some("tool") => {
            part["tool"] = part.get("name").cloned().unwrap_or(Value::Null);
            part["callID"] = part.get("id").cloned().unwrap_or(Value::Null);
            let content = part
                .pointer("/state/content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let output = content
                .iter()
                .filter(|v| v["type"] == "text")
                .filter_map(|v| v["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if part["state"].is_object() {
                part["state"]["output"] = Value::from(output);
                if let Some(error) = part
                    .pointer("/state/error")
                    .filter(|e| !e.is_string())
                    .cloned()
                {
                    part["state"]["error"] = error.get("message").cloned().unwrap_or_else(|| {
                        Value::from(serde_json::to_string_pretty(&error).unwrap_or_default())
                    });
                }
            }
            let complete =
                part.pointer("/state/status").and_then(Value::as_str) == Some("completed");
            let mut parts = vec![part];
            if complete {
                parts.extend(
                    content
                        .iter()
                        .filter(|c| c["type"] == "file")
                        .map(|c| attachment(conn, c)),
                );
            }
            parts
        }
        Some("file") | None if part.get("mime").is_some() => vec![attachment(conn, &part)],
        _ => vec![part],
    }
}

pub(super) const V2_WRITE_ERROR: &str = "OpenCode v2 history is read-only in Sessions Viewer. Rename, delete or restore it in OpenCode; direct projection writes are unsafe for event-sourced sessions.";

pub(super) fn ensure_legacy_write(conn: &Connection, sid: &str) -> Result<(), String> {
    if !has_table(conn, "session")?
        || (has_table(conn, "session_v2")?
            && conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM main.session_v2 WHERE id = ?1)",
                    params![sid],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(db_err)?)
    {
        return Err(V2_WRITE_ERROR.into());
    }
    Ok(())
}

pub(super) fn ensure_legacy_tree_write(conn: &Connection, sid: &str) -> Result<(), String> {
    ensure_legacy_write(conn, sid)?;
    let has_v2 = conn
        .query_row(
            "WITH RECURSIVE target(id, storage) AS (
            SELECT id, storage FROM sv_session WHERE id = ?1 UNION
            SELECT s.id, s.storage FROM sv_session s JOIN target t ON s.parent_id = t.id
         ) SELECT EXISTS(SELECT 1 FROM target WHERE storage = 2)",
            params![sid],
            |r| r.get::<_, bool>(0),
        )
        .map_err(db_err)?;
    if has_v2 {
        return Err(V2_WRITE_ERROR.into());
    }
    Ok(())
}
