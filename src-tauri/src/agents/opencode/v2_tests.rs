//! Synthetic fixtures based on anomalyco/opencode dbcd5ce9485f3338375d1fe9d92864c91e263030:
//! packages/core/src/session/sql.ts and packages/schema/src/session-message.ts.
//! No CLI, real history, environment mutations or network access.
use super::*;
use serde_json::json;

fn v2_db(with_parts: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE project (id TEXT PRIMARY KEY, worktree TEXT NOT NULL, time_updated INTEGER NOT NULL);
         CREATE TABLE session_v2 (
            id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT,
            slug TEXT NOT NULL, title TEXT, directory TEXT NOT NULL,
            time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_archived INTEGER,
            tokens_input INTEGER NOT NULL DEFAULT 0, tokens_output INTEGER NOT NULL DEFAULT 0,
            tokens_reasoning INTEGER NOT NULL DEFAULT 0, tokens_cache_read INTEGER NOT NULL DEFAULT 0,
            tokens_cache_write INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE session_message (
            id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL,
            time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL
         );
         INSERT INTO project VALUES ('p2', 'X:/OpenCodeData/demo', 1000);
         INSERT INTO session_v2 VALUES ('ses_v2','p2',NULL,'mint-moon',NULL,'X:/OpenCodeData/demo',1000,2000,NULL,100,20,5,30,10);",
    ).unwrap();
    if with_parts {
        conn.execute_batch(
            "CREATE TABLE session_part (
                message_id TEXT NOT NULL, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
                type TEXT NOT NULL, call_id TEXT, time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL, data TEXT NOT NULL,
                PRIMARY KEY(message_id, ordinal)
             );",
        )
        .unwrap();
    }
    prepare_read_views(&conn).unwrap();
    conn
}

fn message(conn: &Connection, id: &str, kind: &str, seq: i64, time: i64, data: Value) {
    conn.execute(
        "INSERT INTO session_message VALUES (?1,'ses_v2',?2,?3,?4,?4,?5)",
        params![id, kind, seq, time, data.to_string()],
    )
    .unwrap();
}

fn seed(conn: &Connection) {
    // IDs and clocks deliberately disagree with seq: replay ordering is not wall-clock ordering.
    message(
        conn,
        "msg_z",
        "user",
        1,
        2000,
        json!({"text":"你好 OpenCode v2", "time":{"created":2000}}),
    );
    message(
        conn,
        "msg_a",
        "assistant",
        2,
        1000,
        json!({
            "model":{"id":"test-model","providerID":"local"}, "agent":"plan",
            "tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":30,"write":10}},
            "cost":0.125,
            "content":[
                {"type":"reasoning","text":"Think first"},
                {"type":"text","text":"你好！"},
                {"type":"tool","id":"call_v2","name":"bash","state":{
                    "status":"completed","input":{"command":"git status"},
                    "content":[{"type":"text","text":"clean"}]
                }}
            ]
        }),
    );
}

#[test]
fn v2_lists_nullable_title_and_windows_directory() {
    let conn = v2_db(false);
    seed(&conn);
    let projects = list_projects_impl(&conn, false).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].display_path, "X:/OpenCodeData/demo");
    let page = list_sessions_impl(&conn, "p2", 0, 10, false).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.sessions[0].title, "mint-moon");
    assert_eq!(page.sessions[0].path, "opencode://ses_v2");
    assert_eq!(page.sessions[0].message_count, 1);
    assert!(page.sessions[0].size > 0);
}

#[test]
fn v2_inline_content_maps_roles_order_model_and_tool_results() {
    let conn = v2_db(false);
    seed(&conn);
    let msgs = read(&conn, "ses_v2").unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[0].uuid.as_deref(), Some("msg_z"));
    assert_eq!(msgs[0].blocks[0].text.as_deref(), Some("你好 OpenCode v2"));
    let assistant = &msgs[1];
    assert_eq!(assistant.role, "assistant");
    assert_eq!(assistant.model.as_deref(), Some("test-model"));
    assert_eq!(
        assistant
            .blocks
            .iter()
            .map(|b| b.kind.as_str())
            .collect::<Vec<_>>(),
        ["thinking", "text", "tool_use", "tool_result"]
    );
    assert_eq!(assistant.blocks[2].tool_id.as_deref(), Some("call_v2"));
    assert_eq!(assistant.blocks[3].text.as_deref(), Some("clean"));
    // Export consumes exactly these shared serializable messages.
    let exported = serde_json::to_value(&msgs).unwrap();
    assert_eq!(exported[1]["model"], "test-model");
    let turns = read_turns_impl(&conn, "ses_v2").unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].calls[0].usage.total, 165);
    assert_eq!(turns[0].calls[0].cost_usd, 0.125);
    assert_eq!(turns[0].calls[0].model, "test-model");
    assert!(turns[0].calls[0].has_plan_mode);
    assert_eq!(turns[0].calls[0].bash_commands, ["git"]);
    assert_eq!(
        last_user_text_sql(&conn, "ses_v2").as_deref(),
        Some("你好 OpenCode v2")
    );
    assert_eq!(usage_summary_impl(&conn, "ses_v2").unwrap().total, 165);
    assert_eq!(context_usage_impl(&conn, "ses_v2").unwrap().total, 165);
}

fn split_part(conn: &Connection, ordinal: i64, data: Value) {
    conn.execute(
        "INSERT INTO session_part VALUES ('msg_a','ses_v2',?1,'text',NULL,5000,5000,?2)",
        params![ordinal, data.to_string()],
    )
    .unwrap();
}

#[test]
fn v2_split_parts_override_inline_and_follow_ordinal_not_time_or_type() {
    let conn = v2_db(true);
    seed(&conn);
    split_part(&conn, 2, json!({"type":"text","text":"second"}));
    split_part(&conn, 1, json!({"type":"reasoning","text":"first"}));
    let msgs = read(&conn, "ses_v2").unwrap();
    assert_eq!(msgs[1].blocks.len(), 2);
    assert_eq!(msgs[1].blocks[0].kind, "thinking");
    assert_eq!(msgs[1].blocks[0].text.as_deref(), Some("first"));
    assert_eq!(msgs[1].blocks[1].text.as_deref(), Some("second"));
    assert!(
        !contains_text_impl(&conn, "ses_v2", "clean"),
        "stale inline tool result must not leak"
    );
}

#[test]
fn v2_empty_parts_table_keeps_inline_history() {
    let conn = v2_db(true);
    seed(&conn);
    assert_eq!(read(&conn, "ses_v2").unwrap()[1].blocks.len(), 4);
}

#[test]
fn v2_search_matches_real_user_prompts_not_assistant_or_system() {
    let conn = v2_db(true);
    seed(&conn);
    message(
        &conn,
        "msg_system",
        "system",
        3,
        3000,
        json!({"text":"secret-system-needle"}),
    );
    assert!(contains_text_impl(&conn, "ses_v2", "opencode v2"));
    assert!(contains_text_impl(&conn, "ses_v2", "你好"));
    assert!(!contains_text_impl(&conn, "ses_v2", "not-present"));
    let key = format!("opencode://test-{}", uuid::Uuid::new_v4());
    let hit =
        super::super::find_text_hit(|_| read(&conn, "ses_v2"), &key, 1, "opencode v2").unwrap();
    assert_eq!(hit.msg_uuid.as_deref(), Some("msg_z"));
    assert!(
        super::super::find_text_hit(|_| read(&conn, "ses_v2"), &key, 1, "think first").is_none()
    );
    assert!(super::super::find_text_hit(
        |_| read(&conn, "ses_v2"),
        &key,
        1,
        "secret-system-needle"
    )
    .is_none());
    assert_eq!(
        read(&conn, "ses_v2").unwrap()[2].meta_kind.as_deref(),
        Some("system")
    );
}

#[test]
fn v2_children_and_archived_sessions_keep_existing_list_and_stats_rules() {
    let conn = v2_db(true);
    seed(&conn);
    conn.execute_batch(
        "INSERT INTO session_v2 SELECT 'ses_child',project_id,'ses_v2',slug,'child',directory,
        time_created,2500,NULL,0,0,0,0,0 FROM session_v2 WHERE id='ses_v2';
        INSERT INTO session_v2 SELECT 'ses_arch',project_id,NULL,slug,'archived',directory,
        time_created,3000,4000,0,0,0,0,0 FROM session_v2 WHERE id='ses_v2';",
    )
    .unwrap();
    assert_eq!(
        list_projects_impl(&conn, false).unwrap()[0].session_count,
        1
    );
    assert_eq!(list_projects_impl(&conn, true).unwrap()[0].session_count, 2);
    let page = list_sessions_impl(&conn, "p2", 0, 1, true).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.sessions[0].id, "ses_arch");
    assert_eq!(
        list_sessions_impl(&conn, "p2", 1, 1, true)
            .unwrap()
            .sessions[0]
            .id,
        "ses_v2"
    );
    assert!(list_sessions_impl(&conn, "p2", 50, 1, true)
        .unwrap()
        .sessions
        .is_empty());
    assert_eq!(stats_sessions_impl(&conn, "p2").unwrap().len(), 3);
    assert_eq!(
        companion_sessions_impl(&conn, "ses_v2").unwrap()[0].id,
        "ses_child"
    );
}

#[test]
fn v2_bad_json_and_unknown_records_do_not_fabricate_user_messages_or_break_search() {
    let conn = v2_db(true);
    seed(&conn);
    message(
        &conn,
        "msg_unknown",
        "idle",
        3,
        3000,
        json!({"text":"not a prompt"}),
    );
    message(&conn, "msg_broken", "user", 4, 4000, json!({}));
    conn.execute(
        "UPDATE session_message SET data = '{broken' WHERE id = 'msg_broken'",
        [],
    )
    .unwrap();
    message(
        &conn,
        "msg_badparts",
        "assistant",
        5,
        5000,
        json!({"content":[null,42,"bad",{"type":"text","text":"still readable"}]}),
    );
    split_part(&conn, 1, json!({"type":"text","text":"valid split"}));
    conn.execute(
        "INSERT INTO session_part VALUES ('msg_a','ses_v2',2,'text',NULL,1,1,'{broken')",
        [],
    )
    .unwrap();
    let msgs = read(&conn, "ses_v2").unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[1].blocks[0].text.as_deref(), Some("valid split"));
    assert_eq!(msgs[2].blocks[0].text.as_deref(), Some("still readable"));
    assert_eq!(
        last_user_text_sql(&conn, "ses_v2").as_deref(),
        Some("你好 OpenCode v2")
    );
    assert!(contains_text_impl(&conn, "ses_v2", "readable"));
    assert!(list_sessions_impl(&conn, "p2", 0, 10, false).is_ok());
}

#[test]
fn v2_tool_error_objects_and_streaming_inputs_are_not_lost() {
    let conn = v2_db(true);
    seed(&conn);
    split_part(
        &conn,
        0,
        json!({"type":"tool","id":"bad","name":"bash","state":{
            "status":"error","input":{"command":"false"},"error":{"type":"tool.error","message":"exit 1"}
        }}),
    );
    split_part(
        &conn,
        1,
        json!({"type":"tool","id":"running","name":"read","state":{
            "status":"streaming","input":"{\"filePath\":"
        }}),
    );
    let msgs = read(&conn, "ses_v2").unwrap();
    let blocks = &msgs[1].blocks;
    assert_eq!(blocks.len(), 3);
    assert!(blocks[1].is_error);
    assert_eq!(blocks[1].text.as_deref(), Some("exit 1"));
    assert_eq!(blocks[2].tool_name.as_deref(), Some("read"));
    assert!(blocks[2]
        .tool_input
        .as_deref()
        .unwrap()
        .contains("filePath"));
}

#[test]
fn v2_user_and_tool_images_resolve_inline_and_blob_without_external_io() {
    let conn = v2_db(true);
    seed(&conn);
    let hash = "a".repeat(64);
    conn.execute_batch("CREATE TABLE blob (hash TEXT PRIMARY KEY, mime TEXT, data BLOB);")
        .unwrap();
    conn.execute(
        "INSERT INTO blob VALUES (?1,'image/png',?2)",
        params![hash, b"test-image".to_vec()],
    )
    .unwrap();
    let media = format!("media:{hash}");
    conn.execute("UPDATE session_message SET data = ?1 WHERE id = 'msg_z'", params![json!({"text":"look",
        "files":[{"mime":"image/png","data":"AAAA","name":"inline.png","source":{"type":"inline"}},
                 {"mime":"image/png","uri":media,"name":"stored.png","source":{"type":"inline"}},
                 {"mime":"application/pdf","data":"AA==","name":"doc.pdf","source":{"type":"inline"}}]
    }).to_string()]).unwrap();
    split_part(
        &conn,
        0,
        json!({"type":"tool","id":"image","name":"read","state":{
            "status":"completed","input":{},"content":[{"type":"text","text":"done"},
                {"type":"file","uri":media,"mime":"image/png","name":"output.png"}]
        }}),
    );
    let msgs = read(&conn, "ses_v2").unwrap();
    assert_eq!(
        msgs[0].blocks[1].image_src.as_deref(),
        Some("data:image/png;base64,AAAA")
    );
    assert_eq!(
        msgs[0].blocks[2].image_src.as_deref(),
        Some("data:image/png;base64,dGVzdC1pbWFnZQ==")
    );
    assert_eq!(msgs[0].blocks[3].file_path.as_deref(), Some("doc.pdf"));
    assert_eq!(msgs[1].blocks[2].image_src, msgs[0].blocks[2].image_src);
}

#[test]
fn v2_missing_media_stays_a_file_reference_and_does_not_abort_transcript() {
    let conn = v2_db(false);
    seed(&conn);
    conn.execute("UPDATE session_message SET data = ?1 WHERE id = 'msg_z'", params![json!({"text":"look",
        "files":[{"mime":"image/png","uri":format!("media:{}", "f".repeat(64)),"name":"missing.png"}]
    }).to_string()]).unwrap();
    let msgs = read(&conn, "ses_v2").unwrap();
    assert_eq!(msgs[0].blocks[1].kind, "file");
    assert_eq!(msgs[0].blocks[1].file_path.as_deref(), Some("missing.png"));
}

fn add_legacy(conn: &Connection) {
    conn.execute_batch("CREATE TABLE session AS SELECT * FROM session_v2 WHERE 0;
        CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
        CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
        INSERT INTO session SELECT 'ses_legacy',project_id,NULL,'legacy','old title',directory,500,600,NULL,0,0,0,0,0
          FROM session_v2 WHERE id='ses_v2';
        INSERT INTO message VALUES ('msg_old','ses_legacy',500,'{\"role\":\"user\"}');
        INSERT INTO part VALUES ('prt_old','msg_old','ses_legacy',500,'{\"type\":\"text\",\"text\":\"legacy prompt\"}');").unwrap();
    prepare_read_views(conn).unwrap();
}

#[test]
fn mixed_layout_preserves_legacy_history_and_deduplicates_migrated_ids() {
    let conn = v2_db(true);
    seed(&conn);
    add_legacy(&conn);
    conn.execute_batch("INSERT INTO session SELECT * FROM session_v2 WHERE id='ses_v2';
        INSERT INTO message VALUES ('msg_stale','ses_v2',500,'{\"role\":\"user\"}');
        INSERT INTO part VALUES ('prt_stale','msg_stale','ses_v2',500,'{\"type\":\"text\",\"text\":\"stale prompt\"}');").unwrap();
    let page = list_sessions_impl(&conn, "p2", 0, 10, false).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(
        list_projects_impl(&conn, false).unwrap()[0].session_count,
        2
    );
    assert_eq!(stats_sessions_impl(&conn, "p2").unwrap().len(), 2);
    assert_eq!(
        read(&conn, "ses_legacy").unwrap()[0].blocks[0]
            .text
            .as_deref(),
        Some("legacy prompt")
    );
    assert_eq!(read(&conn, "ses_v2").unwrap().len(), 2);
    assert!(!contains_text_impl(&conn, "ses_v2", "stale prompt"));
    assert_eq!(usage_summary_impl(&conn, "ses_v2").unwrap().total, 165);
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("sv-opencode-v2-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn v2_write_guards_leave_both_layouts_and_trash_destination_untouched() {
    let mut conn = v2_db(true);
    seed(&conn);
    add_legacy(&conn);
    let scratch = Scratch::new();
    let dump = scratch.0.join("trash.jsonl");
    assert_eq!(
        rename_impl(&conn, "ses_v2", "unsafe").unwrap_err(),
        storage::V2_WRITE_ERROR
    );
    assert_eq!(
        soft_delete_impl(&mut conn, "ses_v2", &dump).unwrap_err(),
        storage::V2_WRITE_ERROR
    );
    assert!(!dump.exists());
    rename_impl(&conn, "ses_legacy", "renamed").unwrap();
    soft_delete_impl(&mut conn, "ses_legacy", &dump).unwrap();
    restore_impl(&mut conn, &dump).unwrap();
    assert_eq!(
        list_sessions_impl(&conn, "p2", 0, 10, false).unwrap().total,
        2
    );
    // A legacy dump must never overwrite a now-migrated v2 ID.
    fs::write(
        &dump,
        "{\"kind\":\"opencode-session-row\",\"row\":{\"id\":\"ses_v2\"}}\n",
    )
    .unwrap();
    assert_eq!(
        restore_impl(&mut conn, &dump).unwrap_err(),
        storage::V2_WRITE_ERROR
    );
    assert_eq!(read(&conn, "ses_v2").unwrap().len(), 2);
    assert_eq!(
        conn.query_row("SELECT title FROM session WHERE id='ses_legacy'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "renamed"
    );
}

#[test]
fn legacy_parent_with_v2_child_cannot_be_trashed_partially() {
    let mut conn = v2_db(false);
    add_legacy(&conn);
    conn.execute(
        "UPDATE session_v2 SET parent_id='ses_legacy' WHERE id='ses_v2'",
        [],
    )
    .unwrap();
    let scratch = Scratch::new();
    let dump = scratch.0.join("trash.jsonl");
    assert_eq!(
        soft_delete_impl(&mut conn, "ses_legacy", &dump).unwrap_err(),
        storage::V2_WRITE_ERROR
    );
    assert!(!dump.exists());
    assert_eq!(stats_sessions_impl(&conn, "p2").unwrap().len(), 2);
}

#[test]
fn v2_compaction_usage_is_counted_but_not_used_as_context_size() {
    let conn = v2_db(false);
    seed(&conn);
    message(
        &conn,
        "msg_compaction",
        "compaction",
        3,
        3000,
        json!({"status":"completed","summary":"short summary",
        "model":{"id":"compact-model","providerID":"local"}, "cost":0.01, "tokens":{"input":10,"output":5}}),
    );
    let turns = read_turns_impl(&conn, "ses_v2").unwrap();
    assert_eq!(turns[0].calls.len(), 2);
    assert_eq!(turns[0].calls[1].model, "compact-model");
    assert_eq!(turns[0].calls[1].usage.total, 15);
    assert_eq!(context_usage_impl(&conn, "ses_v2").unwrap().total, 165);
}

#[test]
fn v2_read_only_connection_only_creates_temporary_views_and_preserves_database_bytes() {
    let scratch = Scratch::new();
    let db = scratch.0.join("opencode.db");
    let conn = v2_db(true);
    seed(&conn);
    conn.execute("VACUUM main INTO ?1", params![db.to_string_lossy()])
        .unwrap();
    drop(conn);
    let before = fs::read(&db).unwrap();
    let modified = fs::metadata(&db).unwrap().modified().unwrap();
    let conn = Connection::open_with_flags(
        &db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    prepare_read_views(&conn).unwrap();
    assert_eq!(
        list_projects_impl(&conn, false).unwrap()[0].session_count,
        1
    );
    assert_eq!(
        list_sessions_impl(&conn, "p2", 0, 10, false).unwrap().total,
        1
    );
    assert_eq!(read(&conn, "ses_v2").unwrap().len(), 2);
    assert!(contains_text_impl(&conn, "ses_v2", "你好"));
    assert_eq!(
        last_user_text_sql(&conn, "ses_v2").as_deref(),
        Some("你好 OpenCode v2")
    );
    assert_eq!(usage_summary_impl(&conn, "ses_v2").unwrap().total, 165);
    assert_eq!(context_usage_impl(&conn, "ses_v2").unwrap().total, 165);
    assert_eq!(read_turns_impl(&conn, "ses_v2").unwrap()[0].calls.len(), 1);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM main.sqlite_master WHERE name LIKE 'sv_%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    assert_eq!(fs::read(&db).unwrap(), before);
    assert_eq!(fs::metadata(&db).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
}

#[test]
fn docs_public_fixture_opencode_v2_uses_upstream_columns_and_shared_read_contract() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../../site-docs/public/examples/opencode-v2.sql"
    ))
    .unwrap();
    prepare_read_views(&conn).unwrap();
    let projects = list_projects_impl(&conn, false).unwrap();
    assert_eq!(projects[0].display_path, "X:/synthetic/project");
    let page = list_sessions_impl(&conn, "synthetic-project", 0, 10, false).unwrap();
    assert_eq!(page.sessions[0].title, "synthetic-moon");
    assert_eq!(page.sessions[0].message_count, 1);
    let msgs = read(&conn, "ses_...").unwrap();
    assert_eq!(msgs[0].blocks[0].text.as_deref(), Some("Question"));
    assert_eq!(msgs[1].blocks[0].text.as_deref(), Some("Answer"));
    assert_eq!(
        read_turns_impl(&conn, "ses_...").unwrap()[0].calls[0].cost_usd,
        0.25
    );
    assert_eq!(usage_summary_impl(&conn, "ses_...").unwrap().total, 15);
    assert_eq!(context_usage_impl(&conn, "ses_...").unwrap().total, 15);
}

#[test]
fn unknown_schema_reports_actionable_error_without_creating_tables() {
    let conn = Connection::open_in_memory().unwrap();
    assert!(prepare_read_views(&conn)
        .unwrap_err()
        .contains("Unsupported opencode schema"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
