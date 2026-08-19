use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use toml_edit::{Array, ArrayOfTables, Document, InlineTable, Item, Table, Value as TomlValue};

#[derive(Serialize, Deserialize, Clone)]
pub struct TerminalTurnPayload {
    pub agent: String,
    pub path: String,
    pub state: String,
    #[serde(default = "default_turn_signal_source")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopTask {
    pub agent: String,
    pub path: String,
    pub state: String,
    pub title: String,
    pub updated_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPetResolvedSession {
    pub project_key: String,
    pub session: crate::types::SessionMeta,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHookInstallResult {
    pub claude_settings_path: String,
    pub codex_hooks_path: String,
    pub agy_hooks_path: String,
    pub grok_config_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHookEventStatus {
    pub name: String,
    pub installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHookEntry {
    pub event: String,
    pub category: Option<String>,
    pub matcher: Option<String>,
    pub hook_type: String,
    pub detail: String,
    pub managed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHookAgentStatus {
    pub installed: bool,
    pub config_path: String,
    pub events: Vec<TurnHookEventStatus>,
    pub hooks: Vec<TurnHookEntry>,
}

#[derive(Serialize)]
pub struct TurnHookStatus {
    pub enabled: bool,
    pub claude: TurnHookAgentStatus,
    pub codex: TurnHookAgentStatus,
    pub agy: TurnHookAgentStatus,
    pub grok: TurnHookAgentStatus,
}

const CLAUDE_TURN_HOOKS: [(&str, &str, Option<&str>); 5] = [
    ("UserPromptSubmit", "started", None),
    ("Stop", "completed", None),
    ("StopFailure", "failed", None),
    (
        "Notification",
        "blocked",
        Some("permission_prompt|elicitation_dialog|agent_needs_input"),
    ),
    ("PermissionRequest", "blocked", None),
];

const CODEX_TURN_HOOKS: [(&str, &str); 3] = [
    ("UserPromptSubmit", "started"),
    ("Stop", "completed"),
    ("PermissionRequest", "blocked"),
];

const AGY_TURN_HOOKS: [(&str, &str); 2] = [("PreInvocation", "started"), ("Stop", "completed")];

const GROK_TURN_HOOKS: [(&str, &str, Option<&str>); 6] = [
    ("UserPromptSubmit", "started", None),
    ("Stop", "completed", None),
    ("StopFailure", "failed", None),
    ("StopCancelled", "completed", None),
    ("Notification", "completed", Some("idle_prompt")),
    ("Notification", "blocked", Some("permission_prompt")),
];

fn default_turn_signal_source() -> String {
    "hook".to_string()
}

struct SignalState {
    _watcher: RecommendedWatcher,
    path: PathBuf,
    offset: u64,
}

static SIGNAL_STATE: OnceLock<Mutex<Option<SignalState>>> = OnceLock::new();
static DESKTOP_TASKS: OnceLock<Mutex<HashMap<String, DesktopTask>>> = OnceLock::new();
static PENDING_GROK_SIGNALS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static GROK_CONFIG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn signal_state() -> &'static Mutex<Option<SignalState>> {
    SIGNAL_STATE.get_or_init(|| Mutex::new(None))
}

fn desktop_tasks() -> &'static Mutex<HashMap<String, DesktopTask>> {
    DESKTOP_TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending_grok_signals() -> &'static Mutex<HashSet<String>> {
    PENDING_GROK_SIGNALS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn grok_config_lock() -> &'static Mutex<()> {
    GROK_CONFIG_LOCK.get_or_init(|| Mutex::new(()))
}

fn normalized_session_path(path: &str) -> String {
    let path = path.trim();
    #[cfg(target_os = "windows")]
    {
        path.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.to_string()
    }
}

fn desktop_task_key(agent: &str, path: &str) -> String {
    format!("{agent}\0{}", normalized_session_path(path))
}

fn task_title(agent: &str, path: &str) -> String {
    let fallback = Path::new(path.trim())
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(path.trim())
        .to_string();
    crate::agents::source(agent)
        .ok()
        .map(|source| source.trash_title(Path::new(path.trim())))
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty() && title != "(untitled session)")
        .unwrap_or(fallback)
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn upsert_desktop_task(
    tasks: &mut HashMap<String, DesktopTask>,
    payload: &TerminalTurnPayload,
    updated_at: u64,
) {
    let path = payload.path.trim().to_string();
    tasks.insert(
        desktop_task_key(&payload.agent, &path),
        DesktopTask {
            agent: payload.agent.clone(),
            title: task_title(&payload.agent, &path),
            path,
            state: payload.state.clone(),
            updated_at,
        },
    );
}

fn acknowledge_desktop_task(
    tasks: &mut HashMap<String, DesktopTask>,
    agent: &str,
    path: &str,
) -> bool {
    let key = desktop_task_key(agent, path);
    if tasks
        .get(&key)
        .is_some_and(|task| matches!(task.state.as_str(), "completed" | "failed"))
    {
        tasks.remove(&key);
        true
    } else {
        false
    }
}

pub fn desktop_task_snapshot() -> Result<Vec<DesktopTask>, String> {
    let mut snapshot = desktop_tasks()
        .lock()
        .map_err(|error| error.to_string())?
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for task in &mut snapshot {
        task.title = task_title(&task.agent, &task.path);
    }
    snapshot.sort_by_key(|task| std::cmp::Reverse(task.updated_at));
    Ok(snapshot)
}

pub fn acknowledge_desktop_task_by_path(agent: &str, path: &str) -> Result<bool, String> {
    let mut tasks = desktop_tasks().lock().map_err(|error| error.to_string())?;
    Ok(acknowledge_desktop_task(&mut tasks, agent, path))
}

pub fn resolve_desktop_pet_session(
    agent: &str,
    path: &str,
) -> Result<Option<DesktopPetResolvedSession>, String> {
    let source = crate::agents::source(agent)?;
    let expected = normalized_session_path(path);
    for project in source.list_projects(true, true)? {
        let page = match source.list_sessions(&project.dir_name, 0, usize::MAX, true, true) {
            Ok(page) => page,
            Err(_) => continue,
        };
        if let Some(session) = page
            .sessions
            .into_iter()
            .find(|session| normalized_session_path(&session.path) == expected)
        {
            return Ok(Some(DesktopPetResolvedSession {
                project_key: project.dir_name,
                session,
            }));
        }
    }
    Ok(None)
}

fn data_dir() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "Cannot locate local data directory".to_string())?;
    Ok(base.join("cc-sessions-viewer"))
}

pub fn signal_file_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("turn-signals.jsonl"))
}

fn hook_script_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("turn-signal-hook.cjs"))
}

fn legacy_hook_script_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("claude-turn-signal-hook.cjs"))
}

pub fn emit_turn_signal(app: &AppHandle, mut payload: TerminalTurnPayload) -> Result<(), String> {
    if !matches!(payload.agent.as_str(), "claude" | "codex" | "agy" | "grok") {
        return Err("Unknown agent".to_string());
    }
    if payload.agent == "grok" && payload.path.trim().is_empty() {
        payload.path = payload
            .session_id
            .as_deref()
            .and_then(|session_id| {
                crate::agents::grok::find_updates_path(session_id, payload.cwd.as_deref())
            })
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        if payload.path.trim().is_empty() {
            let Some(session_id) = payload.session_id.clone() else {
                return Err("Missing Grok session id".to_string());
            };
            let key = format!(
                "{}\0{}\0{}",
                session_id,
                payload.prompt_id.as_deref().unwrap_or(""),
                payload.state
            );
            let should_spawn = pending_grok_signals()
                .lock()
                .map_err(|error| error.to_string())?
                .insert(key.clone());
            if should_spawn {
                let app = app.clone();
                let retry_payload = payload;
                std::thread::spawn(move || {
                    for _ in 0..30 {
                        std::thread::sleep(Duration::from_millis(100));
                        let path = crate::agents::grok::find_updates_path(
                            &session_id,
                            retry_payload.cwd.as_deref(),
                        );
                        if let Some(path) = path {
                            let mut resolved = retry_payload.clone();
                            resolved.path = path.to_string_lossy().into_owned();
                            if let Ok(mut pending) = pending_grok_signals().lock() {
                                pending.remove(&key);
                            }
                            let _ = emit_turn_signal(&app, resolved);
                            return;
                        }
                    }
                    if let Ok(mut pending) = pending_grok_signals().lock() {
                        pending.remove(&key);
                    }
                });
            }
            return Ok(());
        }
    }
    if payload.path.trim().is_empty() {
        return Err("Missing session path".to_string());
    }
    if !matches!(
        payload.state.as_str(),
        "started" | "completed" | "blocked" | "failed"
    ) {
        return Err("Unknown session state".to_string());
    }
    if payload.source != "hook" {
        return Err("Unknown session state source".to_string());
    }
    let mut tasks = desktop_tasks().lock().map_err(|error| error.to_string())?;
    upsert_desktop_task(&mut tasks, &payload, current_timestamp_ms());
    drop(tasks);
    app.emit("terminal-turn://state", payload)
        .map_err(|e| e.to_string())
}

pub fn start_signal_watcher(app: AppHandle) -> Result<(), String> {
    let signal_path = signal_file_path()?;
    if let Some(parent) = signal_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create state directory: {e}"))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&signal_path)
        .map_err(|e| format!("Failed to initialize state file: {e}"))?;

    let offset = fs::metadata(&signal_path).map(|m| m.len()).unwrap_or(0);
    let app_for_cb = app.clone();
    let path_for_cb = signal_path.clone();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(ev) = res else { return };
            if !matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                return;
            }
            process_signal_file(&app_for_cb, &path_for_cb);
        })
        .map_err(|e| format!("Failed to initialize turn signal watcher: {e}"))?;

    watcher
        .watch(&signal_path, RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch state file: {e}"))?;

    let mut slot = signal_state().lock().map_err(|e| e.to_string())?;
    *slot = Some(SignalState {
        _watcher: watcher,
        path: signal_path,
        offset,
    });
    Ok(())
}

fn complete_jsonl_prefix_len(buf: &str) -> usize {
    let newline_prefix_len = buf.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    let tail = &buf[newline_prefix_len..];
    if tail.trim().is_empty() {
        return newline_prefix_len;
    }
    if serde_json::from_str::<Value>(tail.trim()).is_ok() {
        buf.len()
    } else {
        newline_prefix_len
    }
}

fn process_signal_file(app: &AppHandle, path: &Path) {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let file_len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    let offset = {
        let mut guard = match signal_state().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(state) = guard.as_mut() else { return };
        if state.path != path {
            return;
        }
        if file_len < state.offset {
            state.offset = 0;
        }
        state.offset
    };

    if file.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return;
    }
    let consumed = complete_jsonl_prefix_len(&buf);
    if consumed == 0 {
        return;
    }
    if let Ok(mut guard) = signal_state().lock() {
        if let Some(state) = guard.as_mut() {
            if state.path == path {
                state.offset = offset.saturating_add(consumed as u64);
            }
        }
    }

    for line in buf[..consumed].lines() {
        let Ok(payload) = serde_json::from_str::<TerminalTurnPayload>(line) else {
            continue;
        };
        let _ = emit_turn_signal(app, payload);
    }
}

pub fn install_turn_hooks() -> Result<TurnHookInstallResult, String> {
    let signal_path = signal_file_path()?;
    if let Some(parent) = signal_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create state directory: {e}"))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&signal_path)
        .map_err(|e| format!("Failed to initialize state file: {e}"))?;

    let script_path = hook_script_path()?;
    write_hook_script(&script_path)?;
    let legacy_script_path = legacy_hook_script_path()?;

    let (settings_path, codex_hooks_path, agy_hooks_path) = turn_hook_config_paths()?;
    let claude_dir = settings_path
        .parent()
        .ok_or_else(|| "Cannot locate Claude config directory".to_string())?;
    fs::create_dir_all(claude_dir)
        .map_err(|e| format!("Failed to create Claude config directory: {e}"))?;

    let mut settings = read_json_object(&settings_path, "Claude settings.json")?;
    for (event, state, matcher) in CLAUDE_TURN_HOOKS {
        merge_turn_hook(
            &mut settings,
            event,
            state,
            "claude",
            matcher,
            &script_path,
            &legacy_script_path,
            &signal_path,
        );
    }

    let formatted = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&settings_path, format!("{formatted}\n"))
        .map_err(|e| format!("Failed to write Claude config: {e}"))?;

    let codex_dir = codex_hooks_path
        .parent()
        .ok_or_else(|| "Cannot locate Codex config directory".to_string())?;
    fs::create_dir_all(codex_dir)
        .map_err(|e| format!("Failed to create Codex config directory: {e}"))?;
    let mut codex_hooks = read_json_object(&codex_hooks_path, "Codex hooks.json")?;
    for (event, state) in CODEX_TURN_HOOKS {
        merge_turn_hook(
            &mut codex_hooks,
            event,
            state,
            "codex",
            None,
            &script_path,
            &legacy_script_path,
            &signal_path,
        );
    }
    let formatted = serde_json::to_string_pretty(&codex_hooks).map_err(|e| e.to_string())?;
    fs::write(&codex_hooks_path, format!("{formatted}\n"))
        .map_err(|e| format!("Failed to write Codex hooks: {e}"))?;

    let agy_config_dir = agy_hooks_path
        .parent()
        .ok_or_else(|| "Cannot locate Antigravity config directory".to_string())?;
    fs::create_dir_all(agy_config_dir)
        .map_err(|e| format!("Failed to create Antigravity config directory: {e}"))?;
    let mut agy_hooks = read_json_object(&agy_hooks_path, "Antigravity hooks.json")?;
    merge_agy_turn_hooks(&mut agy_hooks, &script_path, &signal_path);
    let formatted = serde_json::to_string_pretty(&agy_hooks).map_err(|e| e.to_string())?;
    fs::write(&agy_hooks_path, format!("{formatted}\n"))
        .map_err(|e| format!("Failed to write Antigravity hooks: {e}"))?;

    let grok_config_path = crate::agents::grok::config_path();
    install_grok_turn_hooks(&grok_config_path, &script_path, &signal_path)?;

    Ok(TurnHookInstallResult {
        claude_settings_path: settings_path.to_string_lossy().to_string(),
        codex_hooks_path: codex_hooks_path.to_string_lossy().to_string(),
        agy_hooks_path: agy_hooks_path.to_string_lossy().to_string(),
        grok_config_path: grok_config_path.to_string_lossy().to_string(),
    })
}

pub fn turn_hook_status() -> Result<TurnHookStatus, String> {
    let (claude_path, codex_path, agy_path) = turn_hook_config_paths()?;
    let script_path = hook_script_path()?;
    let legacy_script_path = legacy_hook_script_path()?;
    let signal_path = signal_file_path()?;
    let script_installed = fs::read_to_string(&script_path).is_ok_and(|raw| raw == HOOK_SCRIPT);

    let claude_config = read_json_object(&claude_path, "Claude settings.json")?;
    let claude_events = CLAUDE_TURN_HOOKS
        .iter()
        .map(|(event, state, matcher)| TurnHookEventStatus {
            name: (*event).to_string(),
            installed: script_installed
                && has_turn_hook(
                    &claude_config,
                    event,
                    state,
                    "claude",
                    *matcher,
                    &script_path,
                    &signal_path,
                ),
        })
        .collect::<Vec<_>>();
    let claude_hooks = collect_grouped_hooks(&claude_config, &script_path, &legacy_script_path);
    let claude = hook_agent_status(claude_path, claude_events, claude_hooks);

    let codex_config = read_json_object(&codex_path, "Codex hooks.json")?;
    let codex_events = CODEX_TURN_HOOKS
        .iter()
        .map(|(event, state)| TurnHookEventStatus {
            name: (*event).to_string(),
            installed: script_installed
                && has_turn_hook(
                    &codex_config,
                    event,
                    state,
                    "codex",
                    None,
                    &script_path,
                    &signal_path,
                ),
        })
        .collect::<Vec<_>>();
    let codex_hooks = collect_grouped_hooks(&codex_config, &script_path, &legacy_script_path);
    let codex = hook_agent_status(codex_path, codex_events, codex_hooks);

    let agy_config = read_json_object(&agy_path, "Antigravity hooks.json")?;
    let agy_events = AGY_TURN_HOOKS
        .iter()
        .map(|(event, state)| TurnHookEventStatus {
            name: (*event).to_string(),
            installed: script_installed
                && has_agy_turn_hook(&agy_config, event, state, &script_path, &signal_path),
        })
        .collect::<Vec<_>>();
    let agy_hooks = collect_agy_hooks(&agy_config, &script_path, &legacy_script_path);
    let agy = hook_agent_status(agy_path, agy_events, agy_hooks);

    let grok_path = crate::agents::grok::config_path();
    let grok_config = read_toml_document(&grok_path, "Grok config.toml")?;
    let grok_events = GROK_TURN_HOOKS
        .iter()
        .map(|(event, state, matcher)| TurnHookEventStatus {
            name: grok_hook_event_label(event, *matcher),
            installed: script_installed
                && has_grok_turn_hook(
                    &grok_config,
                    event,
                    state,
                    *matcher,
                    &script_path,
                    &signal_path,
                ),
        })
        .collect::<Vec<_>>();
    let grok_hooks = collect_grok_hooks(&grok_config, &script_path, &legacy_script_path);
    let grok = hook_agent_status(grok_path, grok_events, grok_hooks);

    Ok(TurnHookStatus {
        enabled: claude.installed && codex.installed && agy.installed && grok.installed,
        claude,
        codex,
        agy,
        grok,
    })
}

fn turn_hook_config_paths() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let home = dirs::home_dir().ok_or_else(|| "Cannot locate home directory".to_string())?;
    let codex_dir = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    Ok((
        home.join(".claude").join("settings.json"),
        codex_dir.join("hooks.json"),
        home.join(".gemini").join("config").join("hooks.json"),
    ))
}

fn hook_agent_status(
    path: PathBuf,
    events: Vec<TurnHookEventStatus>,
    hooks: Vec<TurnHookEntry>,
) -> TurnHookAgentStatus {
    TurnHookAgentStatus {
        installed: events.iter().all(|event| event.installed),
        config_path: path.to_string_lossy().to_string(),
        events,
        hooks,
    }
}

fn collect_grouped_hooks(
    config: &Value,
    script_path: &Path,
    legacy_script_path: &Path,
) -> Vec<TurnHookEntry> {
    let mut entries = Vec::new();
    let Some(events) = config.get("hooks").and_then(Value::as_object) else {
        return entries;
    };
    for (event, groups) in events {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let matcher = value_text(group.get("matcher"));
            let Some(items) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for item in items {
                entries.push(hook_entry(
                    event,
                    None,
                    matcher.clone(),
                    item,
                    script_path,
                    legacy_script_path,
                ));
            }
        }
    }
    sort_hook_entries(&mut entries);
    entries
}

fn collect_agy_hooks(
    config: &Value,
    script_path: &Path,
    legacy_script_path: &Path,
) -> Vec<TurnHookEntry> {
    let mut entries = Vec::new();
    let Some(categories) = config.as_object() else {
        return entries;
    };
    for (category, hooks) in categories {
        let Some(events) = hooks.as_object() else {
            continue;
        };
        for (event, items) in events {
            let Some(items) = items.as_array() else {
                continue;
            };
            for item in items {
                entries.push(hook_entry(
                    event,
                    Some(category.clone()),
                    value_text(item.get("matcher")),
                    item,
                    script_path,
                    legacy_script_path,
                ));
            }
        }
    }
    sort_hook_entries(&mut entries);
    entries
}

fn hook_entry(
    event: &str,
    category: Option<String>,
    matcher: Option<String>,
    item: &Value,
    script_path: &Path,
    legacy_script_path: &Path,
) -> TurnHookEntry {
    let hook_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("hook")
        .to_string();
    let detail = ["command", "prompt", "url"]
        .iter()
        .find_map(|key| value_text(item.get(*key)))
        .unwrap_or_else(|| serde_json::to_string(item).unwrap_or_default());
    TurnHookEntry {
        event: event.to_string(),
        category,
        matcher,
        hook_type,
        detail,
        managed: is_our_hook(item, script_path, legacy_script_path),
    }
}

fn value_text(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => serde_json::to_string(other).ok(),
    })
}

fn sort_hook_entries(entries: &mut [TurnHookEntry]) {
    entries.sort_by(|a, b| {
        a.event
            .cmp(&b.event)
            .then_with(|| a.category.cmp(&b.category))
            .then_with(|| a.matcher.cmp(&b.matcher))
            .then_with(|| a.detail.cmp(&b.detail))
    });
}

fn read_json_object(path: &Path, label: &str) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path).map_err(|e| format!("Failed to read {label}: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    let parsed: Value =
        serde_json::from_str(&raw).map_err(|e| format!("{label} is not valid JSON: {e}"))?;
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(format!("{label} top level must be an object"))
    }
}

#[allow(clippy::too_many_arguments)]
fn merge_turn_hook(
    settings: &mut Value,
    event: &str,
    state: &str,
    agent: &str,
    matcher: Option<&str>,
    script_path: &Path,
    legacy_script_path: &Path,
    signal_path: &Path,
) {
    if !settings.get("hooks").is_some_and(Value::is_object) {
        settings["hooks"] = json!({});
    }
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    let entry = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
    if !entry.is_array() {
        *entry = json!([]);
    }
    let Some(groups) = entry.as_array_mut() else {
        return;
    };

    for group in groups.iter_mut() {
        let Some(items) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        items.retain(|item| !is_our_hook(item, script_path, legacy_script_path));
    }
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    });

    let mut group = json!({
        "hooks": [turn_hook_command(agent, state, script_path, signal_path)]
    });
    if let Some(matcher) = matcher {
        group["matcher"] = json!(matcher);
    }
    groups.push(group);
}

const AGY_HOOK_NAME: &str = "cc-sessions-viewer-turn-status";

fn merge_agy_turn_hooks(config: &mut Value, script_path: &Path, signal_path: &Path) {
    let mut hooks = json!({});
    for (event, state) in AGY_TURN_HOOKS {
        hooks[event] = json!([turn_hook_command("agy", state, script_path, signal_path)]);
    }
    config[AGY_HOOK_NAME] = hooks;
}

fn has_turn_hook(
    config: &Value,
    event: &str,
    state: &str,
    agent: &str,
    matcher: Option<&str>,
    script_path: &Path,
    signal_path: &Path,
) -> bool {
    let expected = turn_hook_command(agent, state, script_path, signal_path);
    config["hooks"][event].as_array().is_some_and(|groups| {
        groups.iter().any(|group| {
            let matcher_matches = match matcher {
                Some(expected) => group["matcher"].as_str() == Some(expected),
                None => group.get("matcher").is_none(),
            };
            matcher_matches
                && group["hooks"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item == &expected))
        })
    })
}

fn has_agy_turn_hook(
    config: &Value,
    event: &str,
    state: &str,
    script_path: &Path,
    signal_path: &Path,
) -> bool {
    let expected = turn_hook_command("agy", state, script_path, signal_path);
    config[AGY_HOOK_NAME][event]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item == &expected))
}

fn turn_hook_command(agent: &str, state: &str, script_path: &Path, signal_path: &Path) -> Value {
    json!({
        "type": "command",
        "command": format!(
            "node {} {} {} {}",
            shell_path_arg(script_path),
            shell_string_arg(agent),
            shell_string_arg(state),
            shell_path_arg(signal_path)
        ),
        "timeout": 5
    })
}

fn is_our_hook(item: &Value, script_path: &Path, legacy_script_path: &Path) -> bool {
    item.get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| {
            command_references_path(command, script_path)
                || command_references_path(command, legacy_script_path)
        })
}

fn command_references_path(command: &str, path: &Path) -> bool {
    let raw = path.to_string_lossy();
    command.contains(raw.as_ref()) || command.contains(&raw.replace('\\', "\\\\"))
}

fn shell_path_arg(value: impl AsRef<Path>) -> String {
    let raw = value.as_ref().to_string_lossy();
    shell_string_arg(&raw)
}

fn shell_string_arg(raw: &str) -> String {
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

fn write_hook_script(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create hook script directory: {e}"))?;
    }
    fs::write(path, HOOK_SCRIPT).map_err(|e| format!("Failed to write hook script: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|e| format!("Failed to read hook script permissions: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .map_err(|e| format!("Failed to set hook script permissions: {e}"))?;
    }
    Ok(())
}

const HOOK_SCRIPT: &str = include_str!("turn_signal_hook.cjs");

fn read_toml_document(path: &Path, label: &str) -> Result<Document, String> {
    if !path.exists() {
        return Ok(Document::new());
    }
    let raw = fs::read_to_string(path).map_err(|e| format!("Failed to read {label}: {e}"))?;
    raw.parse::<Document>()
        .map_err(|e| format!("{label} is not valid TOML: {e}"))
}

fn read_toml_source(path: &Path, label: &str) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(path).map_err(|e| format!("Failed to read {label}: {e}"))
}

fn grok_hook_event_label(event: &str, matcher: Option<&str>) -> String {
    matcher
        .map(|matcher| format!("{event}:{matcher}"))
        .unwrap_or_else(|| event.to_string())
}

fn grok_hook_command(state: &str, script_path: &Path, signal_path: &Path) -> String {
    format!(
        "node {} grok {} {}",
        shell_path_arg(script_path),
        shell_string_arg(state),
        shell_path_arg(signal_path)
    )
}

fn is_grok_hook_handler(value: &TomlValue, script_path: &Path, legacy_script_path: &Path) -> bool {
    let Some(table) = value.as_inline_table() else {
        return false;
    };
    let Some(command) = table.get("command").and_then(TomlValue::as_str) else {
        return false;
    };
    command_references_path(command, script_path)
        || command_references_path(command, legacy_script_path)
}

fn grok_handler_table(command: &str) -> InlineTable {
    let mut table = InlineTable::new();
    table.insert("type", TomlValue::from("command"));
    table.insert("command", TomlValue::from(command));
    table.insert("timeout", TomlValue::from(10));
    table
}

fn merge_grok_hook(
    doc: &mut Document,
    event: &str,
    state: &str,
    matcher: Option<&str>,
    script_path: &Path,
    legacy_script_path: &Path,
    signal_path: &Path,
) {
    let hooks_item = doc
        .as_table_mut()
        .entry("hooks")
        .or_insert(Item::Table(Table::new()));
    if !hooks_item.is_table() {
        *hooks_item = Item::Table(Table::new());
    }
    let Some(hooks) = hooks_item.as_table_mut() else {
        return;
    };
    let item = hooks
        .entry(event)
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    if !item.is_array_of_tables() {
        *item = Item::ArrayOfTables(ArrayOfTables::new());
    }
    let groups = item.as_array_of_tables_mut().expect("hooks array");
    let mut installed = false;
    for group in groups.iter_mut() {
        let matches = matcher
            .map(|expected| group.get("matcher").and_then(Item::as_str) == Some(expected))
            .unwrap_or_else(|| group.get("matcher").is_none());
        if !matches {
            continue;
        }
        if let Some(handlers) = group.get_mut("hooks").and_then(Item::as_array_mut) {
            let mut kept = Array::new();
            for value in handlers.iter() {
                if !is_grok_hook_handler(value, script_path, legacy_script_path) {
                    kept.push(value.clone());
                }
            }
            if !installed {
                kept.push(grok_handler_table(&grok_hook_command(
                    state,
                    script_path,
                    signal_path,
                )));
                installed = true;
            }
            *handlers = kept;
        }
    }
    if !installed {
        let mut group = Table::new();
        if let Some(matcher) = matcher {
            group.insert("matcher", Item::Value(TomlValue::from(matcher)));
        }
        let mut handlers = Array::new();
        handlers.push(grok_handler_table(&grok_hook_command(
            state,
            script_path,
            signal_path,
        )));
        group.insert("hooks", Item::Value(TomlValue::Array(handlers)));
        groups.push(group);
    }
}

fn atomic_write_toml(path: &Path, doc: &Document) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Grok config path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let original_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    if path.exists() {
        let backup = path.with_extension("toml.bak");
        fs::copy(path, backup).map_err(|e| format!("Failed to back up Grok config: {e}"))?;
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let temp = parent.join(format!(
        ".{file_name}.viewer-{}-{}.tmp",
        std::process::id(),
        current_timestamp_ms()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("Failed to create Grok config temp file: {e}"))?;
    let write_result = file
        .write_all(doc.to_string().as_bytes())
        .and_then(|_| file.sync_all());
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(format!("Failed to write Grok config: {error}"));
    }
    drop(file);
    if let Some(permissions) = original_permissions {
        fs::set_permissions(&temp, permissions)
            .map_err(|e| format!("Failed to preserve Grok config permissions: {e}"))?;
    }

    if let Err(first_error) = fs::rename(&temp, path) {
        // std::fs::rename does not replace an existing destination on Windows.
        let replacement_backup = parent.join(format!(
            ".{file_name}.viewer-{}-{}.bak",
            std::process::id(),
            current_timestamp_ms()
        ));
        if !path.exists() || fs::rename(path, &replacement_backup).is_err() {
            let _ = fs::remove_file(&temp);
            return Err(format!("Failed to replace Grok config: {first_error}"));
        }
        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::rename(&replacement_backup, path);
            let _ = fs::remove_file(&temp);
            return Err(format!("Failed to install Grok config: {error}"));
        }
        let _ = fs::remove_file(replacement_backup);
    }
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn install_grok_turn_hooks(
    path: &Path,
    script_path: &Path,
    signal_path: &Path,
) -> Result<(), String> {
    let _guard = grok_config_lock()
        .lock()
        .map_err(|error| format!("Failed to lock Grok config: {error}"))?;
    let legacy = legacy_hook_script_path()?;
    for attempt in 0..2 {
        let source = read_toml_source(path, "Grok config.toml")?;
        let mut doc = source
            .parse::<Document>()
            .map_err(|e| format!("Grok config.toml is not valid TOML: {e}"))?;
        for (event, state, matcher) in GROK_TURN_HOOKS {
            merge_grok_hook(
                &mut doc,
                event,
                state,
                matcher,
                script_path,
                &legacy,
                signal_path,
            );
        }
        if read_toml_source(path, "Grok config.toml")? != source {
            if attempt == 0 {
                continue;
            }
            return Err("Grok config changed while installing hooks; try again".to_string());
        }
        return atomic_write_toml(path, &doc);
    }
    unreachable!()
}

fn has_grok_turn_hook(
    doc: &Document,
    event: &str,
    state: &str,
    matcher: Option<&str>,
    script_path: &Path,
    signal_path: &Path,
) -> bool {
    let Some(groups) = doc
        .as_table()
        .get("hooks")
        .and_then(Item::as_table)
        .and_then(|hooks| hooks.get(event))
        .and_then(Item::as_array_of_tables)
    else {
        return false;
    };
    let expected = grok_hook_command(state, script_path, signal_path);
    groups.iter().any(|group| {
        let matcher_matches = matcher
            .map(|expected| group.get("matcher").and_then(Item::as_str) == Some(expected))
            .unwrap_or_else(|| group.get("matcher").is_none());
        matcher_matches
            && group
                .get("hooks")
                .and_then(Item::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item.as_inline_table()
                            .and_then(|table| table.get("command"))
                            .and_then(TomlValue::as_str)
                            == Some(expected.as_str())
                    })
                })
    })
}

fn collect_grok_hooks(
    doc: &Document,
    script_path: &Path,
    legacy_script_path: &Path,
) -> Vec<TurnHookEntry> {
    let mut entries = Vec::new();
    let Some(hooks) = doc.as_table().get("hooks").and_then(Item::as_table) else {
        return entries;
    };
    for (event, item) in hooks.iter() {
        let Some(groups) = item.as_array_of_tables() else {
            continue;
        };
        for group in groups.iter() {
            let matcher = group
                .get("matcher")
                .and_then(Item::as_str)
                .map(str::to_string);
            let Some(items) = group.get("hooks").and_then(Item::as_array) else {
                continue;
            };
            for value in items.iter() {
                let Some(table) = value.as_inline_table() else {
                    continue;
                };
                let managed = is_grok_hook_handler(value, script_path, legacy_script_path);
                if !managed {
                    // Grok config.toml may contain API keys and arbitrary user
                    // commands; never surface those values in the UI inventory.
                    continue;
                }
                entries.push(TurnHookEntry {
                    event: event.to_string(),
                    category: None,
                    matcher: matcher.clone(),
                    hook_type: table
                        .get("type")
                        .and_then(TomlValue::as_str)
                        .unwrap_or("hook")
                        .to_string(),
                    detail: "Managed status hook".to_string(),
                    managed,
                });
            }
        }
    }
    sort_hook_entries(&mut entries);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(agent: &str, path: &str, state: &str) -> TerminalTurnPayload {
        TerminalTurnPayload {
            agent: agent.to_string(),
            path: path.to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            prompt_id: None,
            session_id: None,
            cwd: None,
        }
    }

    #[test]
    fn desktop_tasks_keep_only_the_latest_state_per_session() {
        let mut tasks = HashMap::new();
        upsert_desktop_task(
            &mut tasks,
            &payload("codex", "/tmp/session-1.jsonl", "completed"),
            10,
        );
        upsert_desktop_task(
            &mut tasks,
            &payload("codex", "/tmp/session-1.jsonl", "started"),
            20,
        );

        assert_eq!(tasks.len(), 1);
        let task = tasks.values().next().unwrap();
        assert_eq!(task.state, "started");
        assert_eq!(task.updated_at, 20);
    }

    #[test]
    fn desktop_task_acknowledgement_only_removes_terminal_activity() {
        let mut tasks = HashMap::new();
        upsert_desktop_task(
            &mut tasks,
            &payload("claude", "/tmp/approval.jsonl", "blocked"),
            10,
        );
        upsert_desktop_task(
            &mut tasks,
            &payload("codex", "/tmp/ready.jsonl", "completed"),
            20,
        );
        upsert_desktop_task(
            &mut tasks,
            &payload("agy", "/tmp/failed.json", "failed"),
            30,
        );

        assert!(!acknowledge_desktop_task(
            &mut tasks,
            "claude",
            "/tmp/approval.jsonl"
        ));
        assert!(acknowledge_desktop_task(
            &mut tasks,
            "codex",
            "/tmp/ready.jsonl"
        ));
        assert!(acknowledge_desktop_task(
            &mut tasks,
            "agy",
            "/tmp/failed.json"
        ));
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks.values().next().unwrap().state, "blocked");
    }

    #[test]
    fn desktop_tasks_separate_agents_with_the_same_session_path() {
        let mut tasks = HashMap::new();
        upsert_desktop_task(
            &mut tasks,
            &payload("claude", "/tmp/shared.jsonl", "blocked"),
            10,
        );
        upsert_desktop_task(
            &mut tasks,
            &payload("codex", "/tmp/shared.jsonl", "failed"),
            20,
        );

        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn desktop_tasks_use_the_session_prompt_as_the_codex_title() {
        let path = std::env::temp_dir().join(format!(
            "cc-sessions-viewer-pet-title-{}-{}.jsonl",
            std::process::id(),
            current_timestamp_ms()
        ));
        std::fs::write(
            &path,
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"实现宠物任务状态"}}
"#,
        )
        .unwrap();
        let mut tasks = HashMap::new();
        upsert_desktop_task(
            &mut tasks,
            &payload("codex", path.to_string_lossy().as_ref(), "completed"),
            10,
        );

        assert_eq!(tasks.values().next().unwrap().title, "实现宠物任务状态");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn hook_merge_replaces_legacy_hook_and_preserves_other_handlers() {
        let script = Path::new("/app/turn-signal-hook.cjs");
        let legacy = Path::new("/app/claude-turn-signal-hook.cjs");
        let signal = Path::new("/app/turn-signals.jsonl");
        let mut config = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [
                        {"type":"command","command":"node /app/claude-turn-signal-hook.cjs completed /tmp/old"},
                        {"type":"command","command":"echo keep-me"}
                    ]
                }]
            }
        });

        merge_turn_hook(
            &mut config,
            "Stop",
            "completed",
            "codex",
            None,
            script,
            legacy,
            signal,
        );

        let groups = config["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["hooks"][0]["command"], "echo keep-me");
        let command = groups[1]["hooks"][0]["command"].as_str().unwrap();
        assert!(command.contains("turn-signal-hook.cjs"));
        assert!(command.contains("\"codex\" \"completed\""));
    }

    #[test]
    fn hook_merge_sets_optional_matcher() {
        let mut config = json!({});
        merge_turn_hook(
            &mut config,
            "Notification",
            "blocked",
            "claude",
            Some("permission_prompt|elicitation_dialog|agent_needs_input"),
            Path::new("/app/turn-signal-hook.cjs"),
            Path::new("/app/claude-turn-signal-hook.cjs"),
            Path::new("/app/turn-signals.jsonl"),
        );
        assert_eq!(
            config["hooks"]["Notification"][0]["matcher"],
            "permission_prompt|elicitation_dialog|agent_needs_input"
        );
    }

    #[test]
    fn hook_status_requires_the_expected_command_and_matcher() {
        let script = Path::new("/app/turn-signal-hook.cjs");
        let legacy = Path::new("/app/claude-turn-signal-hook.cjs");
        let signal = Path::new("/app/turn-signals.jsonl");
        let matcher = "permission_prompt|elicitation_dialog|agent_needs_input";
        let mut config = json!({});
        merge_turn_hook(
            &mut config,
            "Notification",
            "blocked",
            "claude",
            Some(matcher),
            script,
            legacy,
            signal,
        );

        assert!(has_turn_hook(
            &config,
            "Notification",
            "blocked",
            "claude",
            Some(matcher),
            script,
            signal,
        ));
        assert!(!has_turn_hook(
            &config,
            "Notification",
            "completed",
            "claude",
            Some(matcher),
            script,
            signal,
        ));
        assert!(!has_turn_hook(
            &config,
            "Notification",
            "blocked",
            "claude",
            Some("permission_prompt"),
            script,
            signal,
        ));
    }

    #[test]
    fn hook_inventory_lists_external_and_managed_handlers() {
        let script = Path::new("/app/turn-signal-hook.cjs");
        let legacy = Path::new("/app/claude-turn-signal-hook.cjs");
        let signal = Path::new("/app/turn-signals.jsonl");
        let mut config = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{"type":"prompt","prompt":"Check this command"}]
                }]
            }
        });
        merge_turn_hook(
            &mut config,
            "Stop",
            "completed",
            "claude",
            None,
            script,
            legacy,
            signal,
        );

        let entries = collect_grouped_hooks(&config, script, legacy);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].event, "PreToolUse");
        assert_eq!(entries[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(entries[0].hook_type, "prompt");
        assert_eq!(entries[0].detail, "Check this command");
        assert!(!entries[0].managed);
        assert_eq!(entries[1].event, "Stop");
        assert!(entries[1].managed);
    }

    #[test]
    fn managed_hook_detection_accepts_escaped_windows_paths() {
        let script = Path::new(r"C:\Users\test\turn-signal-hook.cjs");
        let item = turn_hook_command(
            "codex",
            "started",
            script,
            Path::new(r"C:\Users\test\turn-signals.jsonl"),
        );
        assert!(is_our_hook(
            &item,
            script,
            Path::new(r"C:\Users\test\claude-turn-signal-hook.cjs"),
        ));
    }

    #[test]
    fn agy_hook_merge_uses_antigravity_schema_and_preserves_other_hooks() {
        let mut config = json!({
            "other-hook": {
                "PreInvocation": [{"type":"command","command":"echo keep-me"}]
            }
        });
        merge_agy_turn_hooks(
            &mut config,
            Path::new("/app/turn-signal-hook.cjs"),
            Path::new("/app/turn-signals.jsonl"),
        );

        assert_eq!(
            config["other-hook"]["PreInvocation"][0]["command"],
            "echo keep-me"
        );
        let hook = &config[AGY_HOOK_NAME];
        assert!(hook.get("PreInvocation").is_some_and(Value::is_array));
        assert!(hook.get("Stop").is_some_and(Value::is_array));
        assert!(hook.get("hooks").is_none());
        assert!(hook["PreInvocation"][0]["command"]
            .as_str()
            .is_some_and(|command| command.contains("\"agy\" \"started\"")));
        assert!(hook["Stop"][0]["command"]
            .as_str()
            .is_some_and(|command| command.contains("\"agy\" \"completed\"")));
        assert!(has_agy_turn_hook(
            &config,
            "PreInvocation",
            "started",
            Path::new("/app/turn-signal-hook.cjs"),
            Path::new("/app/turn-signals.jsonl"),
        ));
        assert!(!has_agy_turn_hook(
            &config,
            "PreInvocation",
            "completed",
            Path::new("/app/turn-signal-hook.cjs"),
            Path::new("/app/turn-signals.jsonl"),
        ));
        let entries = collect_agy_hooks(
            &config,
            Path::new("/app/turn-signal-hook.cjs"),
            Path::new("/app/claude-turn-signal-hook.cjs"),
        );
        assert_eq!(entries.len(), 3);
        assert!(entries
            .iter()
            .any(|entry| entry.category.as_deref() == Some("other-hook") && !entry.managed));
        assert_eq!(entries.iter().filter(|entry| entry.managed).count(), 2);
    }

    #[test]
    fn grok_hook_merge_is_idempotent_and_preserves_unrelated_toml() {
        let script = Path::new("/app/turn-signal-hook.cjs");
        let legacy = Path::new("/app/claude-turn-signal-hook.cjs");
        let signal = Path::new("/app/turn-signals.jsonl");
        let source = r#"# Keep this comment and unrelated settings.
[model]
name = "grok-4"
provider = "custom"

[[hooks.Stop]]
hooks = [{ type = "command", command = "echo keep-this-handler" }]
"#;
        let mut config = source.parse::<Document>().unwrap();
        for (event, state, matcher) in GROK_TURN_HOOKS {
            merge_grok_hook(&mut config, event, state, matcher, script, legacy, signal);
        }
        let first = config.to_string();
        let mut second_config = first.parse::<Document>().unwrap();
        for (event, state, matcher) in GROK_TURN_HOOKS {
            merge_grok_hook(
                &mut second_config,
                event,
                state,
                matcher,
                script,
                legacy,
                signal,
            );
        }
        assert_eq!(first, second_config.to_string());
        assert!(first.contains("Keep this comment"));
        assert!(first.contains("name = \"grok-4\""));
        assert!(first.contains("echo keep-this-handler"));
        for (event, state, matcher) in GROK_TURN_HOOKS {
            assert!(has_grok_turn_hook(
                &second_config,
                event,
                state,
                matcher,
                script,
                signal,
            ));
        }
    }

    #[test]
    fn grok_hook_merge_initializes_an_empty_config_without_panicking() {
        let script = Path::new("/app/turn-signal-hook.cjs");
        let legacy = Path::new("/app/claude-turn-signal-hook.cjs");
        let signal = Path::new("/app/turn-signals.jsonl");
        let mut config = Document::new();

        for (event, state, matcher) in GROK_TURN_HOOKS {
            merge_grok_hook(&mut config, event, state, matcher, script, legacy, signal);
        }

        for (event, state, matcher) in GROK_TURN_HOOKS {
            assert!(has_grok_turn_hook(
                &config, event, state, matcher, script, signal,
            ));
        }
    }

    #[test]
    fn grok_hook_inventory_hides_external_commands() {
        let script = Path::new("/app/turn-signal-hook.cjs");
        let legacy = Path::new("/app/claude-turn-signal-hook.cjs");
        let signal = Path::new("/app/turn-signals.jsonl");
        let mut config = Document::new();
        config["hooks"] = Item::Table(Table::new());
        merge_grok_hook(
            &mut config,
            "Stop",
            "completed",
            None,
            script,
            legacy,
            signal,
        );
        let stop = config["hooks"]["Stop"].as_array_of_tables_mut().unwrap();
        let handlers = stop
            .iter_mut()
            .next()
            .unwrap()
            .get_mut("hooks")
            .unwrap()
            .as_array_mut()
            .unwrap();
        handlers.push(grok_handler_table("node /tmp/unrelated-user-command"));

        let entries = collect_grok_hooks(&config, script, legacy);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].managed);
        assert_eq!(entries[0].detail, "Managed status hook");
        assert!(!entries
            .iter()
            .any(|entry| entry.detail.contains("unrelated")));
    }

    #[test]
    fn grok_hook_status_treats_missing_hooks_as_not_installed() {
        let config = "[model]\nname = \"grok-4\"\n".parse::<Document>().unwrap();
        let script = Path::new("/app/turn-signal-hook.cjs");
        let signal = Path::new("/app/turn-signals.jsonl");

        assert!(!has_grok_turn_hook(
            &config,
            "Stop",
            "completed",
            None,
            script,
            signal,
        ));
        assert!(collect_grok_hooks(
            &config,
            script,
            Path::new("/app/claude-turn-signal-hook.cjs"),
        )
        .is_empty());
    }

    #[test]
    fn atomic_toml_write_creates_a_backup_before_replacement() {
        let path = std::env::temp_dir().join(format!(
            "cc-sessions-viewer-grok-config-{}-{}.toml",
            std::process::id(),
            current_timestamp_ms()
        ));
        let backup = path.with_extension("toml.bak");
        fs::write(&path, "# original\n[model]\nname = \"grok-4\"\n").unwrap();
        let mut doc = Document::new();
        doc["model"] = Item::Table(Table::new());
        doc["model"]["name"] = Item::Value(TomlValue::from("grok-4.1"));
        atomic_write_toml(&path, &doc).unwrap();
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "# original\n[model]\nname = \"grok-4\"\n"
        );
        assert!(fs::read_to_string(&path).unwrap().contains("grok-4.1"));
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(backup);
    }

    #[test]
    fn signal_jsonl_consumption_keeps_partial_line_for_next_event() {
        assert_eq!(complete_jsonl_prefix_len(""), 0);
        assert_eq!(complete_jsonl_prefix_len("{\"a\":1}"), 7);
        assert_eq!(complete_jsonl_prefix_len("{\"a\":"), 0);
        assert_eq!(complete_jsonl_prefix_len("{\"a\":1}\n{\"b\":"), 8);
        assert_eq!(complete_jsonl_prefix_len("{\"a\":1}\n{\"b\":2}"), 15);
        assert_eq!(
            complete_jsonl_prefix_len("{\"a\":\"中\"}\n"),
            "{\"a\":\"中\"}\n".len()
        );
    }
}
