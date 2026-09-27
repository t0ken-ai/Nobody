//! LAN settings and inbox are separate from translation preferences/history.
//! Atomic replacement preserves the last valid document on a failed write.
use super::protocol::FileMeta;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Independent LAN preferences; changing these never rewrites translator settings.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub enabled: bool,
    pub name: String,
    pub receive_dir: PathBuf,
}
/// Identity pin plus display metadata. Only the id grants trust.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedPeer {
    pub id: String,
    pub name: String,
    pub platform: String,
}
/// Bounded local history: text is retained, source paths are not; paths contains
/// only successfully received files. Snapshot serialization truncates text.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub direction: String,
    pub phase: String,
    pub created_at: u64,
    pub bytes: u64,
    pub total: u64,
    pub text: String,
    pub files: Vec<FileMeta>,
    pub paths: Vec<PathBuf>,
    pub error: String,
}
impl Record {
    /// UI refreshes need only 180 characters, not a temporary clone of up to
    /// 1 MiB of text per record. Retain the complete body only in the service;
    /// explicit copy/full-text commands continue reading that original value.
    pub fn preview(&self) -> Self {
        Self {
            id: self.id.clone(),
            peer_id: self.peer_id.clone(),
            peer_name: self.peer_name.clone(),
            direction: self.direction.clone(),
            phase: self.phase.clone(),
            created_at: self.created_at,
            bytes: self.bytes,
            total: self.total,
            text: self.text.chars().take(180).collect(),
            files: self.files.clone(),
            paths: self.paths.clone(),
            error: self.error.clone(),
        }
    }
}
/// Feature-owned directory; all filenames supplied to read/write are internal constants.
#[derive(Clone)]
pub struct Store {
    pub dir: PathBuf,
}
impl Store {
    /// Create only this feature directory; report filesystem errors without resetting data.
    pub fn new(dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self { dir })
    }
    /// Missing data uses defaults; malformed existing data must be surfaced, not erased.
    pub fn read<T: DeserializeOwned>(&self, file: &str) -> Result<Option<T>, String> {
        let path = self.dir.join(file);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| format!("互传配置损坏：{}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    /// Flush a sibling temporary file before atomic replacement of one JSON document.
    pub fn write<T: Serialize + ?Sized>(&self, file: &str, value: &T) -> Result<(), String> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.dir).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut temp, value).map_err(|e| e.to_string())?;
        temp.flush().map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist(self.dir.join(file))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Switch LAN metadata to the stable home directory before opening the service.
/// Copy/validate into a sibling staging directory, then publish in one rename;
/// old data stays available for rollback. Existing destinations are authoritative
/// so clearing records/revoking trust cannot be undone by a later legacy import.
/// Received payloads are never copied: history retains their original paths.
pub fn prepare_location(
    home: &Path,
    legacy: &Path,
    old_default: Option<&Path>,
) -> Result<(PathBuf, PathBuf), String> {
    let root = home.join(".translateme");
    fs::create_dir_all(&root).map_err(|e| format!("无法创建互传数据目录：{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("无法保护互传数据目录：{e}"))?;
    }
    let destination = root.join("transfer");
    let received = root.join("received");
    if destination.exists() {
        if !destination.is_dir() {
            return Err("互传数据目录被同名文件占用。".into());
        }
        return Ok((destination, received));
    }
    let staging = tempfile::Builder::new()
        .prefix(".transfer-migration-")
        .tempdir_in(&root)
        .map_err(|e| format!("无法准备互传迁移：{e}"))?;
    let staged_store = Store::new(staging.path().to_owned())?;
    // Only feature-owned filenames are migrated. Never traverse the old folder
    // recursively, follow unrelated files, or move the user's received payloads.
    if let Some(mut settings) = read_legacy::<Settings>(legacy, "settings.json")? {
        if old_default.is_some_and(|path| settings.receive_dir == path) {
            settings.receive_dir = received.clone();
        }
        staged_store.write("settings.json", &settings)?;
    }
    if let Some(trusted) = read_legacy::<Vec<TrustedPeer>>(legacy, "trusted.json")? {
        staged_store.write("trusted.json", &trusted)?;
    }
    if let Some(records) = read_legacy::<Vec<Record>>(legacy, "inbox.json")? {
        staged_store.write("inbox.json", &records)?;
    }
    // An empty fresh install is published too: the destination itself is the
    // migration marker. Do not use a marker written before the actual data.
    fs::rename(staging.path(), &destination)
        .map_err(|e| format!("无法完成互传目录迁移，旧数据保持不变：{e}"))?;
    Ok((destination, received))
}

/// Read without creating/modifying the legacy directory. A bad document aborts
/// the entire migration instead of silently losing records or saved trust.
fn read_legacy<T: DeserializeOwned>(dir: &Path, file: &str) -> Result<Option<T>, String> {
    Store {
        dir: dir.to_owned(),
    }
    .read(file)
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    /// Real files exercise publication/restart behavior without the user's home,
    /// Keychain, network, or actual inbox. Original received bytes must survive.
    #[test]
    fn migrates_metadata_once_and_leaves_received_files_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = Store::new(temp.path().join("legacy")).unwrap();
        let old_default = temp.path().join("Downloads/TranslateMe");
        fs::create_dir_all(&old_default).unwrap();
        let payload = old_default.join("received.txt");
        fs::write(&payload, "original file").unwrap();
        legacy
            .write(
                "settings.json",
                &Settings {
                    enabled: true,
                    name: "My Mac".into(),
                    receive_dir: old_default.clone(),
                },
            )
            .unwrap();
        legacy
            .write(
                "trusted.json",
                &[TrustedPeer {
                    id: "stable-peer-fingerprint".into(),
                    name: "Other Mac".into(),
                    platform: "macOS".into(),
                }],
            )
            .unwrap();
        legacy
            .write(
                "inbox.json",
                &[Record {
                    id: "transfer-id".into(),
                    peer_id: "stable-peer-fingerprint".into(),
                    peer_name: "Other Mac".into(),
                    direction: "received".into(),
                    phase: "completed".into(),
                    created_at: 1,
                    bytes: 13,
                    total: 13,
                    text: "saved message".into(),
                    files: vec![],
                    paths: vec![payload.clone()],
                    error: String::new(),
                }],
            )
            .unwrap();
        let before = fs::read(legacy.dir.join("inbox.json")).unwrap();
        let (dir, received) =
            prepare_location(temp.path(), &legacy.dir, Some(&old_default)).unwrap();
        let current = Store::new(dir.clone()).unwrap();
        assert_eq!(
            current
                .read::<Settings>("settings.json")
                .unwrap()
                .unwrap()
                .receive_dir,
            received
        );
        let records = current.read::<Vec<Record>>("inbox.json").unwrap().unwrap();
        assert_eq!(records[0].paths, [payload.clone()]);
        assert_eq!(records[0].text, "saved message");
        assert_eq!(
            current
                .read::<Vec<TrustedPeer>>("trusted.json")
                .unwrap()
                .unwrap()[0]
                .id,
            "stable-peer-fingerprint"
        );
        assert_eq!(fs::read(legacy.dir.join("inbox.json")).unwrap(), before);
        assert_eq!(fs::read_to_string(payload).unwrap(), "original file");
        assert!(!received.join("received.txt").exists());
        // New data wins, including deliberate deletion, even with corrupt old data.
        current.write("inbox.json", &Vec::<Record>::new()).unwrap();
        current
            .write("trusted.json", &Vec::<TrustedPeer>::new())
            .unwrap();
        fs::write(legacy.dir.join("trusted.json"), "broken old copy").unwrap();
        assert_eq!(
            prepare_location(temp.path(), &legacy.dir, Some(&old_default))
                .unwrap()
                .0,
            dir
        );
        assert!(current
            .read::<Vec<Record>>("inbox.json")
            .unwrap()
            .unwrap()
            .is_empty());
        assert!(current
            .read::<Vec<TrustedPeer>>("trusted.json")
            .unwrap()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn custom_receive_directory_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = Store::new(temp.path().join("legacy")).unwrap();
        let custom = temp.path().join("chosen-folder");
        legacy
            .write(
                "settings.json",
                &Settings {
                    enabled: false,
                    name: "My PC".into(),
                    receive_dir: custom.clone(),
                },
            )
            .unwrap();
        let (dir, _) = prepare_location(
            temp.path(),
            &legacy.dir,
            Some(&temp.path().join("Downloads/TranslateMe")),
        )
        .unwrap();
        let saved = Store::new(dir)
            .unwrap()
            .read::<Settings>("settings.json")
            .unwrap()
            .unwrap();
        assert_eq!(saved.receive_dir, custom);
        assert!(!saved.enabled);
    }

    #[test]
    fn malformed_legacy_file_never_publishes_partial_migration() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = Store::new(temp.path().join("legacy")).unwrap();
        legacy
            .write(
                "settings.json",
                &Settings {
                    enabled: true,
                    name: "My Mac".into(),
                    receive_dir: temp.path().join("Downloads"),
                },
            )
            .unwrap();
        fs::write(legacy.dir.join("trusted.json"), "corrupt").unwrap();
        assert!(prepare_location(temp.path(), &legacy.dir, None).is_err());
        assert!(!temp.path().join(".translateme/transfer").exists());
        assert_eq!(
            fs::read_to_string(legacy.dir.join("trusted.json")).unwrap(),
            "corrupt"
        );
        assert_eq!(
            fs::read_dir(temp.path().join(".translateme"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn fresh_install_uses_home_without_creating_legacy_directories() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("missing-legacy");
        let (dir, received) = prepare_location(temp.path(), &legacy, None).unwrap();
        assert!(dir.is_dir());
        assert_eq!(received, temp.path().join(".translateme/received"));
        assert!(!legacy.exists());
    }
}

/// Commit without replacing existing files, including a concurrent transfer's
/// same-name result. The temporary file stays owned/cleanable on every failure.
pub fn publish_file(
    mut temp: tempfile::NamedTempFile,
    folder: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    temp.as_file_mut().sync_all().map_err(|e| e.to_string())?;
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 0..10000 {
        let candidate = if n == 0 {
            name.to_owned()
        } else {
            format!(
                "{stem} ({n}){}",
                ext.map(|e| format!(".{e}")).unwrap_or_default()
            )
        };
        let final_path = folder.join(candidate);
        match temp.persist_noclobber(&final_path) {
            Ok(_) => return Ok(final_path),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => temp = e.file,
            Err(e) => return Err(format!("保存接收文件失败：{}", e.error)),
        }
    }
    Err("同名文件过多，请更换接收目录。".into())
}

/// Namespace crash leftovers by installation, not merely by a generic prefix.
pub fn staging_prefix(identity: &str) -> String {
    format!(".translateme-incoming-{identity}-")
}

/// Recover only reserved temporary directories before starting the listener.
/// Symlink entries are skipped, so cleanup never traverses a redirected path.
pub fn cleanup_staging(folder: &Path, identity: &str) -> Result<(), String> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("无法访问接收目录：{e}")),
    };
    let prefix = staging_prefix(identity);
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name().to_string_lossy().starts_with(&prefix)
            && entry.file_type().map_err(|e| e.to_string())?.is_dir()
        {
            fs::remove_dir_all(entry.path()).map_err(|e| format!("清理未完成传输失败：{e}"))?;
        }
    }
    Ok(())
}
