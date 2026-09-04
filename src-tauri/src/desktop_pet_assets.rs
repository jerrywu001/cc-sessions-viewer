use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
#[cfg(target_os = "windows")]
use std::time::SystemTime;

const SPRITESHEET_WIDTH: u32 = 1536;
const V1_SPRITESHEET_HEIGHT: u32 = 1872;
const V2_SPRITESHEET_HEIGHT: u32 = 2288;

const CODEX_PETS: &[(&str, &str, &str, &[u8])] = &[
    (
        "codex",
        "Codex",
        "The original Codex companion.",
        include_bytes!("../assets/desktop-pets/codex/codex/spritesheet.webp"),
    ),
    (
        "dewey",
        "Dewey",
        "A calm companion for focused workspace days",
        include_bytes!("../assets/desktop-pets/codex/dewey/spritesheet.webp"),
    ),
    (
        "fireball",
        "Fireball",
        "Hot path energy for fast iteration.",
        include_bytes!("../assets/desktop-pets/codex/fireball/spritesheet.webp"),
    ),
    (
        "hoots",
        "Hoots",
        "A sharp-eyed owl for polished work in a blink.",
        include_bytes!("../assets/desktop-pets/codex/hoots/spritesheet.webp"),
    ),
    (
        "rocky",
        "Rocky",
        "A steady rock when the diff gets large.",
        include_bytes!("../assets/desktop-pets/codex/rocky/spritesheet.webp"),
    ),
    (
        "seedy",
        "Seedy",
        "Small green shoots for new ideas.",
        include_bytes!("../assets/desktop-pets/codex/seedy/spritesheet.webp"),
    ),
    (
        "stacky",
        "Stacky",
        "A balanced stack for deep work.",
        include_bytes!("../assets/desktop-pets/codex/stacky/spritesheet.webp"),
    ),
    (
        "bsod",
        "BSOD",
        "A tiny blue-screen gremlin.",
        include_bytes!("../assets/desktop-pets/codex/bsod/spritesheet.webp"),
    ),
    (
        "null-signal",
        "Null Signal",
        "Quiet signal from the void.",
        include_bytes!("../assets/desktop-pets/codex/null-signal/spritesheet.webp"),
    ),
];

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPetDefinition {
    key: String,
    id: String,
    display_name: String,
    description: Option<String>,
    sprite_version_number: u8,
    spritesheet_path: String,
    source: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPetCatalog {
    pets: Vec<DesktopPetDefinition>,
    custom_directory: String,
    codex_installed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomPetManifest {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default = "default_sprite_version")]
    sprite_version_number: u8,
    #[serde(default = "default_spritesheet_path")]
    spritesheet_path: String,
}

#[derive(Debug)]
struct AsarEntry {
    offset: u64,
    size: usize,
}

fn default_sprite_version() -> u8 {
    1
}

fn default_spritesheet_path() -> String {
    "spritesheet.webp".to_string()
}

#[tauri::command]
pub fn desktop_pet_catalog(app: tauri::AppHandle) -> Result<DesktopPetCatalog, String> {
    let app_data = crate::app_storage::data_dir(&app)?;
    let imported_directory = app_data.join("desktop-pets").join("codex");
    fs::create_dir_all(&imported_directory).map_err(|error| error.to_string())?;

    let codex_asar = find_codex_asar();
    if let Some(path) = codex_asar.as_deref() {
        if let Err(error) = import_codex_pets(path, &imported_directory) {
            eprintln!("Failed to import Codex desktop pets: {error}");
        }
    }
    install_bundled_codex_pets(&imported_directory)?;

    let custom_directory = custom_pets_directory()?;
    fs::create_dir_all(&custom_directory).map_err(|error| error.to_string())?;

    let mut pets = collect_imported_codex_pets(&imported_directory);
    pets.extend(collect_custom_pets(&custom_directory));

    Ok(DesktopPetCatalog {
        pets,
        custom_directory: custom_directory.to_string_lossy().into_owned(),
        codex_installed: codex_asar.is_some(),
    })
}

#[tauri::command]
pub fn delete_custom_desktop_pet(pet_id: String) -> Result<(), String> {
    let custom_directory = custom_pets_directory()?;
    let pet_directory = find_custom_pet_directory(&custom_directory, &pet_id)?;
    fs::remove_dir_all(pet_directory).map_err(|error| error.to_string())
}

fn custom_pets_directory() -> Result<PathBuf, String> {
    Ok(dirs::home_dir()
        .ok_or_else(|| "Home directory is unavailable".to_string())?
        .join(".codex")
        .join("pets"))
}

fn find_custom_pet_directory(directory: &Path, pet_id: &str) -> Result<PathBuf, String> {
    let pet_id = pet_id.trim();
    if pet_id.is_empty() {
        return Err("Custom pet id is missing".to_string());
    }

    let root = fs::canonicalize(directory).map_err(|error| error.to_string())?;
    let entries = fs::read_dir(&root).map_err(|error| error.to_string())?;
    let mut matched = None;

    for entry in entries.flatten() {
        let pet_directory = entry.path();
        if !pet_directory.is_dir() {
            continue;
        }
        let Ok(canonical_directory) = fs::canonicalize(&pet_directory) else {
            continue;
        };
        // A custom pet is always a direct child of ~/.codex/pets. This blocks
        // symlinked folders from redirecting deletion outside the custom-pet root.
        if canonical_directory.parent() != Some(root.as_path()) {
            continue;
        }

        let Some(folder_id) = pet_directory.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(manifest_bytes) = fs::read(pet_directory.join("pet.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<CustomPetManifest>(&manifest_bytes) else {
            continue;
        };
        let manifest_id = manifest
            .id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| folder_id.to_string());
        if manifest_id == pet_id {
            matched = Some(canonical_directory);
        }
    }

    matched.ok_or_else(|| "Custom pet was not found".to_string())
}

fn install_bundled_codex_pets(output_directory: &Path) -> Result<(), String> {
    for (id, _, _, bundled_spritesheet) in CODEX_PETS {
        validate_spritesheet(bundled_spritesheet, 2)?;

        let pet_directory = output_directory.join(id);
        fs::create_dir_all(&pet_directory).map_err(|error| error.to_string())?;
        let spritesheet = pet_directory.join("spritesheet.webp");
        let existing_is_valid = fs::read(&spritesheet)
            .map(|bytes| validate_spritesheet(&bytes, 2).is_ok())
            .unwrap_or(false);
        if !existing_is_valid {
            fs::write(spritesheet, bundled_spritesheet).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn import_codex_pets(asar_path: &Path, output_directory: &Path) -> Result<(), String> {
    let mut file = File::open(asar_path).map_err(|error| error.to_string())?;
    let (content_start, header) = read_asar_header(&mut file)?;

    for (id, _, _, _) in CODEX_PETS {
        let Some(metadata) = find_spritesheet_entry(&header, id) else {
            continue;
        };
        let entry = parse_asar_entry(metadata)?;
        let bytes = read_asar_entry(&mut file, content_start, &entry)?;
        validate_spritesheet(&bytes, 2)?;

        let pet_directory = output_directory.join(id);
        fs::create_dir_all(&pet_directory).map_err(|error| error.to_string())?;
        fs::write(pet_directory.join("spritesheet.webp"), bytes)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn find_spritesheet_entry<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    let files = node.get("files").and_then(Value::as_object)?;
    let prefix = format!("{id}-spritesheet-");
    for (name, metadata) in files {
        if name.starts_with(&prefix) && name.ends_with(".webp") {
            return Some(metadata);
        }
    }
    for metadata in files.values() {
        if let Some(found) = find_spritesheet_entry(metadata, id) {
            return Some(found);
        }
    }
    None
}

fn collect_imported_codex_pets(directory: &Path) -> Vec<DesktopPetDefinition> {
    CODEX_PETS
        .iter()
        .filter_map(|(id, display_name, description, _)| {
            let spritesheet = directory.join(id).join("spritesheet.webp");
            let bytes = fs::read(&spritesheet).ok()?;
            validate_spritesheet(&bytes, 2).ok()?;
            Some(DesktopPetDefinition {
                key: format!("codex:{id}"),
                id: (*id).to_string(),
                display_name: (*display_name).to_string(),
                description: Some((*description).to_string()),
                sprite_version_number: 2,
                spritesheet_path: spritesheet.to_string_lossy().into_owned(),
                source: "codex".to_string(),
            })
        })
        .collect()
}

fn collect_custom_pets(directory: &Path) -> Vec<DesktopPetDefinition> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut pets = BTreeMap::new();
    for entry in entries.flatten() {
        let pet_directory = entry.path();
        if !pet_directory.is_dir() {
            continue;
        }
        let Some(folder_id) = pet_directory.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(manifest_bytes) = fs::read(pet_directory.join("pet.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<CustomPetManifest>(&manifest_bytes) else {
            continue;
        };
        if !matches!(manifest.sprite_version_number, 1 | 2)
            || !is_safe_relative_path(&manifest.spritesheet_path)
        {
            continue;
        }
        let spritesheet = pet_directory.join(&manifest.spritesheet_path);
        let Ok(bytes) = fs::read(&spritesheet) else {
            continue;
        };
        if validate_spritesheet(&bytes, manifest.sprite_version_number).is_err() {
            continue;
        }

        let id = manifest
            .id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| folder_id.to_string());
        let display_name = manifest
            .display_name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        pets.insert(
            id.clone(),
            DesktopPetDefinition {
                key: format!("custom:{id}"),
                id,
                display_name,
                description: manifest.description,
                sprite_version_number: manifest.sprite_version_number,
                spritesheet_path: spritesheet.to_string_lossy().into_owned(),
                source: "custom".to_string(),
            },
        );
    }
    pets.into_values().collect()
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.trim().is_empty()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn read_asar_header<R: Read>(reader: &mut R) -> Result<(u64, Value), String> {
    let mut prelude = [0_u8; 16];
    reader
        .read_exact(&mut prelude)
        .map_err(|error| error.to_string())?;
    let header_size = u32::from_le_bytes(prelude[4..8].try_into().unwrap()) as u64;
    let json_size = u32::from_le_bytes(prelude[12..16].try_into().unwrap()) as usize;
    if json_size == 0 || json_size > 64 * 1024 * 1024 {
        return Err("Invalid app.asar header size".to_string());
    }
    let mut json = vec![0_u8; json_size];
    reader
        .read_exact(&mut json)
        .map_err(|error| error.to_string())?;
    let header = serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    Ok((8 + header_size, header))
}

fn parse_asar_entry(value: &Value) -> Result<AsarEntry, String> {
    let offset = value
        .get("offset")
        .and_then(Value::as_str)
        .ok_or_else(|| "app.asar entry offset is missing".to_string())?
        .parse::<u64>()
        .map_err(|error| error.to_string())?;
    let size = value
        .get("size")
        .and_then(Value::as_u64)
        .and_then(|size| usize::try_from(size).ok())
        .ok_or_else(|| "app.asar entry size is invalid".to_string())?;
    Ok(AsarEntry { offset, size })
}

fn read_asar_entry<R: Read + Seek>(
    reader: &mut R,
    content_start: u64,
    entry: &AsarEntry,
) -> Result<Vec<u8>, String> {
    reader
        .seek(SeekFrom::Start(content_start + entry.offset))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0_u8; entry.size];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn validate_spritesheet(bytes: &[u8], sprite_version: u8) -> Result<(), String> {
    let expected_height = match sprite_version {
        1 => V1_SPRITESHEET_HEIGHT,
        2 => V2_SPRITESHEET_HEIGHT,
        _ => return Err(format!("Unsupported sprite version {sprite_version}")),
    };
    match image_dimensions(bytes) {
        Some((SPRITESHEET_WIDTH, height)) if height == expected_height => Ok(()),
        Some((width, height)) => Err(format!(
            "Unsupported pet spritesheet size {width}x{height}; expected {SPRITESHEET_WIDTH}x{expected_height}"
        )),
        None => Err("Pet spritesheet must be a valid PNG or WebP image".to_string()),
    }
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    png_dimensions(bytes).or_else(|| webp_dimensions(bytes))
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 20 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut position = 12_usize;
    while position.checked_add(8)? <= bytes.len() {
        let chunk_size =
            u32::from_le_bytes(bytes[position + 4..position + 8].try_into().ok()?) as usize;
        let data_start = position + 8;
        let data_end = data_start.checked_add(chunk_size)?;
        if data_end > bytes.len() {
            return None;
        }
        let dimensions = match &bytes[position..position + 4] {
            b"VP8X" if chunk_size >= 10 => Some((
                1 + u32::from(bytes[data_start + 4])
                    + (u32::from(bytes[data_start + 5]) << 8)
                    + (u32::from(bytes[data_start + 6]) << 16),
                1 + u32::from(bytes[data_start + 7])
                    + (u32::from(bytes[data_start + 8]) << 8)
                    + (u32::from(bytes[data_start + 9]) << 16),
            )),
            b"VP8 "
                if chunk_size >= 10
                    && &bytes[data_start + 3..data_start + 6] == b"\x9d\x01\x2a" =>
            {
                Some((
                    u16::from_le_bytes(bytes[data_start + 6..data_start + 8].try_into().ok()?)
                        as u32
                        & 0x3fff,
                    u16::from_le_bytes(bytes[data_start + 8..data_start + 10].try_into().ok()?)
                        as u32
                        & 0x3fff,
                ))
            }
            b"VP8L" if chunk_size >= 5 && bytes[data_start] == 0x2f => {
                let bits =
                    u32::from_le_bytes(bytes[data_start + 1..data_start + 5].try_into().ok()?);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            _ => None,
        };
        if dimensions.is_some() {
            return dimensions;
        }
        position = data_end + (chunk_size & 1);
    }
    None
}

#[cfg(target_os = "windows")]
fn find_codex_asar() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles", "ProgramW6432"] {
        let Some(root) = std::env::var_os(variable) else {
            continue;
        };
        let windows_apps = PathBuf::from(root).join("WindowsApps");
        let Ok(entries) = fs::read_dir(windows_apps) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with("OpenAI.Codex_")
            {
                continue;
            }
            let asar = entry.path().join("app").join("resources").join("app.asar");
            if asar.is_file() {
                candidates.push(asar);
            }
        }
    }
    candidates.into_iter().max_by_key(|path| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    })
}

#[cfg(target_os = "macos")]
fn find_codex_asar() -> Option<PathBuf> {
    fn app_asar(app: impl AsRef<Path>) -> PathBuf {
        app.as_ref()
            .join("Contents")
            .join("Resources")
            .join("app.asar")
    }

    let mut candidates = vec![
        app_asar("/Applications/ChatGPT.app"),
        app_asar("/Applications/Codex.app"),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(app_asar(home.join("Applications").join("ChatGPT.app")));
        candidates.push(app_asar(home.join("Applications").join("Codex.app")));
        candidates.push(
            home.join("Library")
                .join("Application Support")
                .join("codex-plusplus")
                .join("backup")
                .join("app.asar"),
        );
        candidates.push(app_asar(
            home.join("Library")
                .join("Application Support")
                .join("codex-plusplus")
                .join("backup")
                .join("Codex.app"),
        ));
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn find_codex_asar() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn vp8x(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        for value in [width - 1, height - 1] {
            bytes.push((value & 0xff) as u8);
            bytes.push(((value >> 8) & 0xff) as u8);
            bytes.push(((value >> 16) & 0xff) as u8);
        }
        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        bytes
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\0\0\0\0\0".to_vec();
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn reads_an_asar_header_and_entry() {
        let header = serde_json::json!({
            "files": { "pet.webp": { "size": 4, "offset": "0" } }
        });
        let json = serde_json::to_vec(&header).unwrap();
        let header_size = (8 + json.len()) as u32;
        let mut asar = vec![0_u8; 16];
        asar[4..8].copy_from_slice(&header_size.to_le_bytes());
        asar[12..16].copy_from_slice(&(json.len() as u32).to_le_bytes());
        asar.extend_from_slice(&json);
        asar.extend_from_slice(b"TEST");

        let mut cursor = Cursor::new(asar);
        let (content_start, parsed) = read_asar_header(&mut cursor).unwrap();
        let entry = parse_asar_entry(&parsed["files"]["pet.webp"]).unwrap();
        assert_eq!(
            read_asar_entry(&mut cursor, content_start, &entry).unwrap(),
            b"TEST"
        );
    }

    #[test]
    fn finds_codex_spritesheets_in_nested_asar_assets() {
        let header = serde_json::json!({
            "files": {
                "webview": {
                    "files": {
                        "assets": {
                            "files": {
                                "codex-spritesheet-v6-BRBFriCM.webp": {
                                    "size": 42,
                                    "offset": "7"
                                }
                            }
                        }
                    }
                }
            }
        });
        let entry = find_spritesheet_entry(&header, "codex").unwrap();
        assert_eq!(entry.get("size").and_then(Value::as_u64), Some(42));
        assert!(find_spritesheet_entry(&header, "bsod").is_none());
    }

    #[test]
    fn validates_codex_v1_and_v2_png_or_webp_sheets() {
        assert!(validate_spritesheet(&vp8x(1536, 1872), 1).is_ok());
        assert!(validate_spritesheet(&vp8x(1536, 2288), 2).is_ok());
        assert!(validate_spritesheet(&png(1536, 1872), 1).is_ok());
        assert!(validate_spritesheet(&png(1536, 2288), 2).is_ok());
        assert!(validate_spritesheet(&vp8x(1536, 2288), 1).is_err());
        assert!(validate_spritesheet(&png(192, 208), 2).is_err());
    }

    #[test]
    fn custom_manifest_uses_codex_defaults() {
        let manifest: CustomPetManifest = serde_json::from_str("{}").unwrap();
        assert_eq!(manifest.sprite_version_number, 1);
        assert_eq!(manifest.spritesheet_path, "spritesheet.webp");
        assert!(manifest.id.is_none());
        assert!(is_safe_relative_path("art/pet.png"));
        assert!(!is_safe_relative_path("../pet.png"));
    }

    #[test]
    fn installs_all_bundled_codex_pets_without_codex_desktop() {
        let directory = std::env::temp_dir().join(format!(
            "cc-sessions-viewer-bundled-pets-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);

        install_bundled_codex_pets(&directory).unwrap();

        let pets = collect_imported_codex_pets(&directory);
        assert_eq!(pets.len(), CODEX_PETS.len());
        for (id, _, _, bundled_spritesheet) in CODEX_PETS {
            let installed = fs::read(directory.join(id).join("spritesheet.webp")).unwrap();
            assert_eq!(installed, *bundled_spritesheet);
        }

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn deletes_only_the_matching_direct_custom_pet_directory() {
        let directory = std::env::temp_dir().join(format!(
            "cc-sessions-viewer-custom-pet-delete-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("porter")).unwrap();
        fs::create_dir_all(directory.join("keep-me")).unwrap();
        fs::write(
            directory.join("porter").join("pet.json"),
            r#"{"id":"porter","spritesheetPath":"spritesheet.webp"}"#,
        )
        .unwrap();
        fs::write(
            directory.join("keep-me").join("pet.json"),
            r#"{"id":"keep-me","spritesheetPath":"spritesheet.webp"}"#,
        )
        .unwrap();

        let target = find_custom_pet_directory(&directory, "porter").unwrap();
        assert_eq!(
            target.file_name().and_then(|name| name.to_str()),
            Some("porter")
        );
        assert!(find_custom_pet_directory(&directory, "../keep-me").is_err());

        fs::remove_dir_all(target).unwrap();
        assert!(!directory.join("porter").exists());
        assert!(directory.join("keep-me").exists());

        let _ = fs::remove_dir_all(directory);
    }
}
