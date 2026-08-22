use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{SessionMeta, SessionPage};
use crate::util::{home, is_jsonl, mtime_millis};

fn bookmarks_path() -> PathBuf {
    home()
        .join(".claude")
        .join(".session-viewer-bookmarks.json")
}

type BookmarksMap = HashMap<String, Vec<String>>;

fn read_all() -> BookmarksMap {
    let p = bookmarks_path();
    if !p.exists() {
        return BookmarksMap::new();
    }
    fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<BookmarksMap>(&s).ok())
        .unwrap_or_default()
}

fn write_all(map: &BookmarksMap) -> Result<(), String> {
    let p = bookmarks_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let json =
        serde_json::to_string_pretty(map).map_err(|e| format!("Failed to serialize: {e}"))?;
    fs::write(&p, json).map_err(|e| format!("Failed to write bookmarks: {e}"))
}

pub fn load(agent: &str) -> Vec<String> {
    read_all().remove(agent).unwrap_or_default()
}

pub fn add(agent: &str, path: &str) -> Result<(), String> {
    let mut map = read_all();
    let list = map.entry(agent.to_string()).or_default();
    if !list.iter().any(|p| p == path) {
        list.push(path.to_string());
    }
    write_all(&map)
}

pub fn remove(agent: &str, path: &str) -> Result<(), String> {
    let mut map = read_all();
    if let Some(list) = map.get_mut(agent) {
        list.retain(|p| p != path);
    }
    write_all(&map)
}

pub fn count_sessions(dir: &Path) -> (usize, u64) {
    let mut count = 0usize;
    let mut last = 0u64;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let fp = e.path();
            if is_jsonl(&fp) {
                count += 1;
                let mt = mtime_millis(&fp);
                if mt > last {
                    last = mt;
                }
            }
        }
    }
    (count, last)
}

pub fn list_sessions_in_dir(dir: &str, offset: usize, limit: usize) -> Result<SessionPage, String> {
    let p = Path::new(dir);
    if !p.is_dir() {
        return Ok(SessionPage {
            total: 0,
            sessions: vec![],
        });
    }
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let fp = e.path();
            if is_jsonl(&fp) {
                let mt = mtime_millis(&fp);
                files.push((fp, mt));
            }
        }
    }
    files.sort_by_key(|b| std::cmp::Reverse(b.1));
    let total = files.len();
    let window = files.into_iter().skip(offset).take(limit);
    let sessions = window
        .map(|(fp, mt)| {
            let file_name = fp
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let id = file_name.trim_end_matches(".jsonl").to_string();
            let size = fp.metadata().map(|m| m.len()).unwrap_or(0);
            SessionMeta {
                id,
                file_name: file_name.clone(),
                path: fp.to_string_lossy().to_string(),
                title: file_name.trim_end_matches(".jsonl").to_string(),
                cwd: Some(dir.to_string()),
                created: None,
                modified: mt,
                size,
                message_count: 0,
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
