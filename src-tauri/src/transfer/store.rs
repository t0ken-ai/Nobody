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
