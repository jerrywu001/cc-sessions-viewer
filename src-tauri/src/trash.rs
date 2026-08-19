// 回收站 —— agent 无关。
// 所有 agent 共用一个 trash 目录：~/.claude/.session-viewer-trash/。
//
// Claude / Codex / agy 的一个会话仍是单个 JSONL；Grok 的一个会话则是包含
// summary.json、updates.jsonl 等文件的目录。meta 旁车明确记录 storageKind、原 storage
// root 和正文相对路径，使删除 / 恢复 / 永久删除都以完整会话为原子。

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::agents::{self, SessionStorageKind, SessionStorageUnit};
use crate::types::TrashItem;
use crate::util::{home, is_jsonl, now_millis};

pub fn trash_dir() -> PathBuf {
    let directory = home().join(".claude").join(".session-viewer-trash");
    let _ = fs::create_dir_all(&directory);
    directory
}

fn validate_trash_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Trash item name cannot be empty".to_string());
    }
    let path = Path::new(name);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err("Invalid trash item name".to_string());
    }
    Ok(())
}

fn validate_relative_entry(path: &Path) -> Result<(), String> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Invalid transcript path in trash metadata".to_string());
    }
    Ok(())
}

fn unique_trash_name(directory: &Path, base: &str) -> String {
    let now = now_millis();
    for suffix in 0u32.. {
        let name = if suffix == 0 {
            format!("{now}-{base}")
        } else {
            format!("{now}-{suffix}-{base}")
        };
        if !directory.join(&name).exists() && !directory.join(format!("{name}.meta")).exists() {
            return name;
        }
    }
    unreachable!("unbounded trash suffix search")
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

fn copy_directory(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "refusing to copy a symlink from a session directory: {}",
                    source_path.display()
                ),
            ));
        }
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn move_storage(source: &Path, destination: &Path, kind: SessionStorageKind) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "Destination already exists: {}",
            destination.display()
        ));
    }
    if fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    match kind {
        SessionStorageKind::File => fs::copy(source, destination)
            .and_then(|_| fs::remove_file(source))
            .map(|_| ())
            .map_err(|error| format!("Failed to move session to trash: {error}")),
        SessionStorageKind::Directory => {
            if let Err(error) = copy_directory(source, destination) {
                let _ = fs::remove_dir_all(destination);
                return Err(format!("Failed to copy session directory: {error}"));
            }
            if let Err(error) = fs::remove_dir_all(source) {
                return Err(format!(
                    "Copied session directory but failed to remove the source; both copies were kept: {error}"
                ));
            }
            Ok(())
        }
    }
}

fn remove_storage(path: &Path, kind: SessionStorageKind) -> Result<(), String> {
    match kind {
        SessionStorageKind::File => {
            fs::remove_file(path).map_err(|error| format!("Failed to delete permanently: {error}"))
        }
        SessionStorageKind::Directory => fs::remove_dir_all(path)
            .map_err(|error| format!("Failed to delete session directory permanently: {error}")),
    }
}

fn write_meta(path: &Path, value: &Value) -> Result<(), String> {
    let temporary =
        path.with_extension(format!("meta.tmp-{}-{}", std::process::id(), now_millis()));
    fs::write(&temporary, value.to_string())
        .map_err(|error| format!("Failed to write trash metadata: {error}"))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Failed to install trash metadata: {error}"));
    }
    Ok(())
}

fn metadata_for(
    agent: &str,
    project_label: &str,
    deleted_at: u64,
    unit: &SessionStorageUnit,
) -> Value {
    serde_json::json!({
        "agent": agent,
        "originalPath": unit.entry_path().to_string_lossy(),
        "originalRootPath": unit.root_path.to_string_lossy(),
        "entryRelativePath": unit.entry_relative_path.to_string_lossy(),
        "storageKind": unit.kind.as_str(),
        "projectLabel": project_label,
        "deletedAt": deleted_at,
    })
}

fn soft_delete_in(
    directory: &Path,
    agent: &str,
    path: &str,
    project_label: &str,
) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create trash directory: {error}"))?;

    if agent == "opencode" && agents::opencode::is_virtual_path(path) {
        let session_id = path.trim_start_matches("opencode://");
        let base = if session_id.is_empty() {
            "session.jsonl".to_string()
        } else {
            format!("{session_id}.jsonl")
        };
        let deleted_at = now_millis();
        let trash_name = unique_trash_name(directory, &base);
        let destination = directory.join(&trash_name);
        agents::opencode::soft_delete_to_trash(path, &destination)?;
        let meta = serde_json::json!({
            "agent": agent,
            "originalPath": path,
            "originalRootPath": path,
            "entryRelativePath": "",
            "storageKind": "file",
            "projectLabel": project_label,
            "deletedAt": deleted_at,
        });
        if let Err(error) = write_meta(&directory.join(format!("{trash_name}.meta")), &meta) {
            // Put the database rows back before returning so a metadata write
            // failure never turns soft-delete into data loss.
            let _ = agents::opencode::restore_from_trash(&destination);
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        return Ok(());
    }

    let source = agents::source(agent)?;
    let unit = source.session_storage_unit(Path::new(path))?;
    if !unit.root_path.exists() {
        return Err("Session storage does not exist".to_string());
    }
    let base = unit
        .root_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            if unit.kind == SessionStorageKind::Directory {
                "grok-session".to_string()
            } else {
                "session.jsonl".to_string()
            }
        });
    let deleted_at = now_millis();
    let trash_name = unique_trash_name(directory, &base);
    let destination = directory.join(&trash_name);
    let meta_path = directory.join(format!("{trash_name}.meta"));
    let meta = metadata_for(agent, project_label, deleted_at, &unit);

    // Install metadata first. If the move then fails, remove the sidecar; if the
    // process exits after the move, the item remains recoverable.
    write_meta(&meta_path, &meta)?;
    if let Err(error) = move_storage(&unit.root_path, &destination, unit.kind) {
        // A cross-filesystem fallback can finish the copy but fail while
        // removing the source. Keep metadata whenever the destination exists;
        // preserving a recoverable duplicate is safer than orphaning it.
        if !destination.exists() {
            let _ = fs::remove_file(&meta_path);
        }
        return Err(error);
    }
    Ok(())
}

pub fn soft_delete(agent: &str, path: &str, project_label: &str) -> Result<(), String> {
    soft_delete_in(&trash_dir(), agent, path, project_label)
}

fn read_meta(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null)
}

fn storage_kind_from_meta(meta: &Value, data_path: &Path) -> SessionStorageKind {
    meta.get("storageKind")
        .and_then(Value::as_str)
        .map(SessionStorageKind::from_str)
        .unwrap_or_else(|| {
            if data_path.is_dir() {
                SessionStorageKind::Directory
            } else {
                SessionStorageKind::File
            }
        })
}

fn trashed_entry_path(
    data_path: &Path,
    meta: &Value,
    kind: SessionStorageKind,
) -> Result<PathBuf, String> {
    if kind == SessionStorageKind::File {
        return Ok(data_path.to_path_buf());
    }
    let relative = meta
        .get("entryRelativePath")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("updates.jsonl"));
    validate_relative_entry(&relative)?;
    Ok(data_path.join(relative))
}

fn list_in(directory: &Path) -> Result<Vec<TrashItem>, String> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    let entries =
        fs::read_dir(directory).map_err(|error| format!("Failed to read trash: {error}"))?;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let data_path = entry.path();
        let trash_file = entry.file_name().to_string_lossy().to_string();
        if trash_file.ends_with(".meta") || trash_file.contains(".meta.tmp-") {
            continue;
        }
        let meta_path = directory.join(format!("{trash_file}.meta"));
        if fs::symlink_metadata(&meta_path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue;
        }
        let meta = read_meta(&meta_path);
        let kind = storage_kind_from_meta(&meta, &data_path);
        if kind == SessionStorageKind::File && !is_jsonl(&data_path) {
            continue;
        }
        if kind == SessionStorageKind::Directory && !data_path.is_dir() {
            continue;
        }
        let agent = meta
            .get("agent")
            .and_then(Value::as_str)
            .unwrap_or("claude")
            .to_string();
        let original_path = meta
            .get("originalPath")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let project_label = meta
            .get("projectLabel")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let deleted_at = meta.get("deletedAt").and_then(Value::as_u64).unwrap_or(0);
        let entry_path = trashed_entry_path(&data_path, &meta, kind)?;
        let title = agents::source(&agent)
            .map(|source| source.trash_title(&entry_path))
            .unwrap_or_default();
        output.push(TrashItem {
            trash_file,
            agent,
            project_label,
            original_path,
            trash_path: entry_path.to_string_lossy().to_string(),
            deleted_at,
            title,
            size: directory_size(&data_path),
        });
    }
    output.sort_by_key(|item| std::cmp::Reverse(item.deleted_at));
    Ok(output)
}

pub fn list() -> Result<Vec<TrashItem>, String> {
    list_in(&trash_dir())
}

fn restore_in(directory: &Path, trash_file: &str) -> Result<(), String> {
    validate_trash_name(trash_file)?;
    let source_path = directory.join(trash_file);
    let meta_path = directory.join(format!("{trash_file}.meta"));
    for (path, label) in [(&source_path, "trash item"), (&meta_path, "trash metadata")] {
        if fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(format!("Refusing to restore a symlinked {label}"));
        }
    }
    let raw = fs::read_to_string(&meta_path)
        .map_err(|_| "Missing metadata — cannot determine restore location".to_string())?;
    let meta: Value =
        serde_json::from_str(&raw).map_err(|error| format!("Corrupted metadata: {error}"))?;
    let original_path = meta
        .get("originalPath")
        .and_then(Value::as_str)
        .ok_or("Metadata missing original path")?;
    let agent = meta
        .get("agent")
        .and_then(Value::as_str)
        .unwrap_or("claude");
    if agent == "opencode" && agents::opencode::is_virtual_path(original_path) {
        agents::opencode::restore_from_trash(&source_path)?;
        let _ = fs::remove_file(&source_path);
        let _ = fs::remove_file(&meta_path);
        return Ok(());
    }

    let kind = storage_kind_from_meta(&meta, &source_path);
    let original_root = meta
        .get("originalRootPath")
        .and_then(Value::as_str)
        .unwrap_or(original_path);
    let entry_path = PathBuf::from(original_path);
    let root_path = PathBuf::from(original_root);
    let source = agents::source(agent)?;
    source.validate_restore_target(&entry_path, &root_path, kind)?;
    let trashed_entry = trashed_entry_path(&source_path, &meta, kind)?;
    if !trashed_entry.exists() {
        return Err("Trash item is missing its transcript".to_string());
    }
    if root_path.exists() {
        return Err(format!(
            "Restore destination already exists: {}",
            root_path.display()
        ));
    }
    if let Some(parent) = root_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create restore directory: {error}"))?;
    }
    move_storage(&source_path, &root_path, kind)
        .map_err(|error| format!("Failed to restore session: {error}"))?;
    let _ = fs::remove_file(&meta_path);
    Ok(())
}

pub fn restore(trash_file: &str) -> Result<(), String> {
    restore_in(&trash_dir(), trash_file)
}

fn permanent_delete_in(directory: &Path, trash_file: &str) -> Result<(), String> {
    validate_trash_name(trash_file)?;
    let data_path = directory.join(trash_file);
    let meta_path = directory.join(format!("{trash_file}.meta"));
    for (path, label) in [(&data_path, "trash item"), (&meta_path, "trash metadata")] {
        if fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(format!(
                "Refusing to permanently delete a symlinked {label}"
            ));
        }
    }
    let meta = read_meta(&meta_path);
    let kind = storage_kind_from_meta(&meta, &data_path);
    remove_storage(&data_path, kind)?;
    let _ = fs::remove_file(meta_path);
    Ok(())
}

pub fn permanent_delete(trash_file: &str) -> Result<(), String> {
    permanent_delete_in(&trash_dir(), trash_file)
}

fn empty_in(directory: &Path) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    let entries =
        fs::read_dir(directory).map_err(|error| format!("Failed to read trash: {error}"))?;
    let mut first_error: Option<String> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let result = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if let Err(error) = result {
            first_error.get_or_insert_with(|| {
                format!("Failed to empty trash item {}: {error}", path.display())
            });
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub fn empty() -> Result<(), String> {
    empty_in(&trash_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "trash-storage-test-{name}-{}-{}",
            std::process::id(),
            now_millis()
        ))
    }

    #[test]
    fn directory_storage_moves_and_restores_as_one_unit() {
        let root = scratch("roundtrip");
        let source = root.join("source-session");
        let trashed = root.join("trash-session");
        let restored = root.join("restored-session");
        fs::create_dir_all(source.join("terminal")).unwrap();
        fs::write(source.join("summary.json"), "{}").unwrap();
        fs::write(source.join("updates.jsonl"), "{}\n").unwrap();
        fs::write(source.join("terminal").join("output.log"), "hello").unwrap();

        let unit = SessionStorageUnit {
            root_path: source.clone(),
            entry_relative_path: PathBuf::from("updates.jsonl"),
            kind: SessionStorageKind::Directory,
        };
        let meta = metadata_for("grok", "/tmp/project", 123, &unit);

        move_storage(&source, &trashed, SessionStorageKind::Directory).unwrap();
        assert!(!source.exists());
        assert!(trashed.join("summary.json").is_file());
        assert!(trashed.join("updates.jsonl").is_file());
        assert!(trashed.join("terminal").join("output.log").is_file());
        assert_eq!(meta["storageKind"], "directory");
        assert_eq!(meta["entryRelativePath"], "updates.jsonl");
        assert_eq!(
            trashed_entry_path(&trashed, &meta, SessionStorageKind::Directory).unwrap(),
            trashed.join("updates.jsonl")
        );

        move_storage(&trashed, &restored, SessionStorageKind::Directory).unwrap();
        assert!(!trashed.exists());
        assert!(restored.join("summary.json").is_file());
        assert!(restored.join("updates.jsonl").is_file());
        assert!(restored.join("terminal").join("output.log").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restore_does_not_overwrite_an_existing_target() {
        let root = scratch("restore-existing");
        let trash = root.join("trash");
        let destination = root.join("original-session.jsonl");
        fs::create_dir_all(&trash).unwrap();
        fs::write(&destination, "original\n").unwrap();
        fs::write(trash.join("123-session.jsonl"), "trashed\n").unwrap();
        fs::write(
            trash.join("123-session.jsonl.meta"),
            serde_json::json!({
                "agent":"claude",
                "originalPath":destination,
                "originalRootPath":destination,
                "entryRelativePath":"",
                "storageKind":"file",
                "projectLabel":"test",
                "deletedAt":123
            })
            .to_string(),
        )
        .unwrap();

        let error = restore_in(&trash, "123-session.jsonl").unwrap_err();
        assert!(error.contains("already exists"));
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original\n");
        assert!(trash.join("123-session.jsonl").is_file());
        assert!(trash.join("123-session.jsonl.meta").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn trash_names_reject_path_traversal() {
        assert!(validate_trash_name("../session.jsonl").is_err());
        assert!(validate_trash_name("nested/session.jsonl").is_err());
        assert!(validate_trash_name("session.jsonl").is_ok());
    }

    #[test]
    fn relative_entry_rejects_parent_and_absolute_components() {
        assert!(validate_relative_entry(Path::new("updates.jsonl")).is_ok());
        assert!(validate_relative_entry(Path::new("../updates.jsonl")).is_err());
        assert!(validate_relative_entry(Path::new("/tmp/updates.jsonl")).is_err());
    }

    #[test]
    fn permanent_delete_handles_directory_items() {
        let root = scratch("permanent");
        let item = root.join("123-session");
        fs::create_dir_all(&item).unwrap();
        fs::write(item.join("updates.jsonl"), "").unwrap();
        fs::write(
            root.join("123-session.meta"),
            r#"{"storageKind":"directory"}"#,
        )
        .unwrap();
        permanent_delete_in(&root, "123-session").unwrap();
        assert!(!item.exists());
        assert!(!root.join("123-session.meta").exists());
        let _ = fs::remove_dir_all(root);
    }
}
