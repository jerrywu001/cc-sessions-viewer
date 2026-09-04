use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize, Deserialize)]
struct StorageConfig {
    path: PathBuf,
}

fn config_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "Config directory is unavailable".to_string())?;
    Ok(base.join("cc-sessions-viewer").join("storage.json"))
}

fn default_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|error| error.to_string())
}

pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let default = default_dir(app)?;
    let path = config_path()?;
    if !path.is_file() {
        fs::create_dir_all(&default).map_err(|error| error.to_string())?;
        return Ok(default);
    }
    let config: StorageConfig =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("Invalid storage configuration: {error}"))?;
    if config.path.as_os_str().is_empty() || !config.path.is_absolute() {
        return Err("Configured data path must be an absolute path".to_string());
    }
    fs::create_dir_all(&config.path).map_err(|error| error.to_string())?;
    Ok(config.path)
}

fn write_config(path: &Path) -> Result<(), String> {
    let config_path = config_path()?;
    let parent = config_path
        .parent()
        .ok_or_else(|| "Config directory is unavailable".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = config_path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&StorageConfig {
        path: path.to_path_buf(),
    })
    .map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(temporary, config_path).map_err(|error| error.to_string())
}

fn remove_config() -> Result<(), String> {
    let path = config_path()?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn move_entry(source: &Path, destination: &Path) -> io::Result<()> {
    if !destination.exists() {
        match fs::rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                copy_entry(source, destination)?;
                if source.is_dir() {
                    fs::remove_dir_all(source)?;
                } else {
                    fs::remove_file(source)?;
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    if source.is_dir() && destination.is_dir() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            move_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
        fs::remove_dir(source)
    } else {
        if source.is_file() && destination.is_file() && fs::read(source)? == fs::read(destination)? {
            fs::remove_file(source)
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Data conflict at {}", destination.display()),
            ))
        }
    }
}

fn copy_entry(source: &Path, destination: &Path) -> io::Result<()> {
    if source.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(source, destination).map(|_| ())
    }
}

fn migrate(source: &Path, destination: &Path) -> Result<(), String> {
    if source == destination || !source.exists() {
        fs::create_dir_all(destination).map_err(|error| error.to_string())?;
        return Ok(());
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        move_entry(&entry.path(), &destination.join(entry.file_name()))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn data_directory(app: AppHandle) -> Result<String, String> {
    Ok(data_dir(&app)?.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn change_data_directory(app: AppHandle, new_path: String) -> Result<String, String> {
    let destination = PathBuf::from(new_path.trim());
    if destination.as_os_str().is_empty() || !destination.is_absolute() {
        return Err("Data path must be an absolute path".to_string());
    }
    let source = data_dir(&app)?;
    if source != destination {
        let source_cmp = fs::canonicalize(&source).unwrap_or(source.clone());
        let destination_cmp = fs::canonicalize(destination.parent().unwrap_or(&destination))
            .unwrap_or_else(|_| destination.parent().unwrap_or(&destination).to_path_buf())
            .join(destination.file_name().unwrap_or_default());
        if destination_cmp.starts_with(&source_cmp) {
            return Err("Data path cannot be inside the current data directory".to_string());
        }
        migrate(&source, &destination)?;
        write_config(&destination)?;
    }
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn reset_data_directory(app: AppHandle) -> Result<String, String> {
    let source = data_dir(&app)?;
    let destination = default_dir(&app)?;
    if source != destination {
        let source_cmp = fs::canonicalize(&source).unwrap_or(source.clone());
        let destination_cmp = fs::canonicalize(destination.parent().unwrap_or(&destination))
            .unwrap_or_else(|_| destination.parent().unwrap_or(&destination).to_path_buf())
            .join(destination.file_name().unwrap_or_default());
        if destination_cmp.starts_with(&source_cmp) {
            return Err("Default data path cannot be inside the current data directory".to_string());
        }
        migrate(&source, &destination)?;
    }
    remove_config()?;
    Ok(destination.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_merges_directories() {
        let root = std::env::temp_dir().join(format!("storage-test-{}", uuid::Uuid::new_v4()));
        let source = root.join("old");
        let destination = root.join("new");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(destination.join("nested")).unwrap();
        fs::write(source.join("a"), b"a").unwrap();
        fs::write(source.join("nested/b"), b"b").unwrap();
        migrate(&source, &destination).unwrap();
        assert_eq!(fs::read(destination.join("a")).unwrap(), b"a");
        assert_eq!(fs::read(destination.join("nested/b")).unwrap(), b"b");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_file_conflicts() {
        let root = std::env::temp_dir().join(format!("storage-test-{}", uuid::Uuid::new_v4()));
        let source = root.join("old");
        let destination = root.join("new");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("a"), b"a").unwrap();
        fs::write(destination.join("a"), b"b").unwrap();
        assert!(migrate(&source, &destination).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
