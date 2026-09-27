//! Reinstall-stable LLM storage, independent of UI, translation and LAN identity.
//! SQLCipher and its local key live together in ~/.translateme/llm/. The old
//! root-level llm.db remains untouched as a backup; it is never unlocked or
//! imported. Missing/corrupt keys in the new store never reset existing data.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

static STORE: LazyLock<Result<Store, String>> = LazyLock::new(|| {
    let home = dirs::home_dir().ok_or("无法找到用户主目录。")?;
    Ok(Store::at(&home.join(".translateme")))
});

/// Only the last LLM profile is restored. Automatic capture permissions and LAN
/// trust remain in their existing stores; restoring credentials grants no trust.
#[derive(Serialize, Deserialize)]
pub struct Profile {
    pub engine: String,
    pub endpoint: String,
    pub model: String,
    pub prompt: String,
}

/// A NULL row is a deliberate deletion. No legacy credential fallback exists.
pub enum KeyState {
    Missing,
    Saved(Option<String>),
}

struct Store {
    path: PathBuf,
    connection: Mutex<Option<Connection>>,
}

/// Resolve one process-local store without embedding a machine-specific path.
fn store() -> Result<&'static Store, String> {
    STORE.as_ref().map_err(Clone::clone)
}

/// A fresh subdirectory separates the new local store from the unreadable old
/// OS-protected database. Reads never create a key/database; only Save does.
impl Store {
    fn at(root: &Path) -> Self {
        Self {
            path: root.join("llm/llm.db"),
            connection: Mutex::new(None),
        }
    }

    fn access<T>(
        &self,
        create: bool,
        operation: impl FnOnce(Option<&mut Connection>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut connection = self.connection.lock().map_err(|_| "LLM 存储暂不可用。")?;
        let exists = self
            .path
            .try_exists()
            .map_err(|_| "无法检查 LLM 数据库。")?;
        if connection.is_none() && (create || exists) {
            let key = local_key(&self.path, exists)?;
            *connection = Some(open_encrypted(&self.path, &key)?);
        }
        operation(connection.as_mut())
    }
}

/// Persist a random key before creating ciphertext. A complete directory backup
/// is sufficient to reopen; losing just master.key is an error, never a reset.
fn local_key(database: &Path, database_exists: bool) -> Result<String, String> {
    let key_path = database.with_file_name("master.key");
    let existing = crate::private_files::read(&key_path, 64)?;
    let bytes = match existing {
        Some(bytes) => bytes,
        None if database_exists => {
            return Err("LLM 本地解密密钥缺失，请恢复 llm/master.key；数据库保持不变。".into())
        }
        None => {
            let key = rand::random::<[u8; 32]>()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            crate::private_files::write_new(&key_path, key.as_bytes())?;
            // Another process may have published first. Always use the durable
            // winner, never the unpersisted candidate from this process.
            crate::private_files::read(&key_path, 64)?.ok_or("无法读取已保存的本地解密密钥。")?
        }
    };
    if bytes.len() != 64 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err("LLM 本地解密密钥格式异常；原文件保持不变。".into());
    }
    String::from_utf8(bytes).map_err(|_| "LLM 本地解密密钥格式异常。".into())
}

/// Set the key before touching schema, verify the linked library is SQLCipher,
/// then authenticate existing pages. A wrong key/corrupt file must never migrate.
fn open_encrypted(path: &Path, key: &str) -> Result<Connection, String> {
    let parent = path.parent().ok_or("LLM 数据库目录不可用。")?;
    crate::private_files::directory(parent)?;
    // Create the empty DB inside the private directory, then protect it before
    // SQLite writes its first page. Existing symlinks are rejected as well.
    crate::private_files::write_new(path, &[])?;
    crate::private_files::protect(path)?;
    let mut db = Connection::open(path).map_err(|_| "无法打开 LLM 数据库。")?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "无法设置数据库等待时间。")?;
    db.pragma_update(None, "key", key)
        .map_err(|_| "无法设置数据库解密密钥。")?;
    let cipher: String = db
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .map_err(|_| "SQLCipher 未启用，已拒绝写入密钥。")?;
    if cipher.is_empty() {
        return Err("SQLCipher 未启用，已拒绝写入密钥。".into());
    }
    db.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|_| "LLM 数据库无法解锁或文件已损坏；原文件未改动。")?;
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| "无法读取数据库版本。")?;
    if version > 1 {
        return Err("此 LLM 数据库来自较新版本，请升级 TranslateMe。".into());
    }
    // DELETE journals leave a single portable DB after a committed save. SQLite
    // encrypts journal pages too; memory-only temp tables avoid plaintext spills.
    db.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON;",
    )
    .map_err(|_| "无法配置加密数据库。")?;
    if version == 0 {
        let tx = db.transaction().map_err(|_| "无法初始化数据库。")?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS endpoint_keys(endpoint TEXT PRIMARY KEY, api_key TEXT);
            CREATE TABLE IF NOT EXISTS profile(id INTEGER PRIMARY KEY CHECK(id=1), data TEXT NOT NULL);
            PRAGMA user_version=1;").map_err(|_| "无法初始化数据库。")?;
        tx.commit().map_err(|_| "无法保存数据库结构。")?;
    }
    Ok(db)
}

/// Startup restores only nonsecret profile fields to application state. Absent
/// files are normal on first run; malformed stored profiles are explicit errors.
pub fn load_profile() -> Result<Option<Profile>, String> {
    store()?.access(false, |db| {
        let Some(db) = db else { return Ok(None) };
        let raw: Option<String> = db
            .query_row("SELECT data FROM profile WHERE id=1", [], |r| r.get(0))
            .optional()
            .map_err(|_| "无法读取 LLM 配置。")?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(|_| "LLM 配置内容损坏。".into()))
            .transpose()
    })
}

/// Read exactly one normalized endpoint, without exposing the DB or other keys.
pub fn get_key(endpoint: &str) -> Result<KeyState, String> {
    store()?.access(false, |db| match db {
        Some(db) => read_key(db, endpoint),
        None => Ok(KeyState::Missing),
    })
}

/// Preserve the distinction between an unconfigured endpoint and explicit deletion.
fn read_key(db: &Connection, endpoint: &str) -> Result<KeyState, String> {
    let value: Option<Option<String>> = db
        .query_row(
            "SELECT api_key FROM endpoint_keys WHERE endpoint=?1",
            [endpoint],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "无法读取 LLM 密钥。")?;
    Ok(value.map(KeyState::Saved).unwrap_or(KeyState::Missing))
}

/// Profile and credential changes commit together. None preserves an existing
/// endpoint key; Some(empty) records deletion. SQL parameters never enter logs.
pub fn save(profile: Option<&Profile>, key: Option<(&str, &str)>) -> Result<(), String> {
    store()?.access(true, |db| {
        write(db.ok_or("LLM 数据库不可用。")?, profile, key)
    })
}

/// The caller holds the store mutex; one SQLite transaction prevents torn
/// profile/credential pairs if either statement fails or the process exits.
fn write(
    db: &mut Connection,
    profile: Option<&Profile>,
    key: Option<(&str, &str)>,
) -> Result<(), String> {
    let tx = db.transaction().map_err(|_| "无法开始 LLM 配置保存。")?;
    if let Some(profile) = profile {
        let data = serde_json::to_string(profile).map_err(|_| "无法编码 LLM 配置。")?;
        tx.execute("INSERT INTO profile(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data", [data])
            .map_err(|_| "无法保存 LLM 配置。")?;
    }
    if let Some((endpoint, key)) = key {
        let key = (!key.is_empty()).then_some(key);
        tx.execute("INSERT INTO endpoint_keys(endpoint,api_key) VALUES(?1,?2) ON CONFLICT(endpoint) DO UPDATE SET api_key=excluded.api_key", params![endpoint, key])
            .map_err(|_| "无法保存 LLM 密钥。")?;
    }
    tx.commit().map_err(|_| "无法提交 LLM 配置。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Tests use isolated files, never the user's real data directory.
    #[test]
    fn encrypted_file_reopens_and_wrong_key_does_not_modify_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("llm.db");
        let key = local_key(&path, false).unwrap();
        let mut db = open_encrypted(&path, &key).unwrap();
        let profile = Profile {
            engine: "llm".into(),
            endpoint: "https://example.test/v1".into(),
            model: "test-model".into(),
            prompt: "private instructions".into(),
        };
        write(
            &mut db,
            Some(&profile),
            Some((&profile.endpoint, "secret-test-key")),
        )
        .unwrap();
        drop(db);
        let before = fs::read(&path).unwrap();
        for plaintext in [
            "SQLite format 3",
            "secret-test-key",
            "private instructions",
            "example.test",
        ] {
            assert!(!before
                .windows(plaintext.len())
                .any(|p| p == plaintext.as_bytes()));
        }
        assert!(open_encrypted(&path, "wrong credential").is_err());
        assert_eq!(before, fs::read(&path).unwrap());
        // Simulate reinstall: in-memory state is gone; the DB + local key survive.
        let mut reopened = open_encrypted(&path, &key).unwrap();
        assert!(
            matches!(read_key(&reopened, &profile.endpoint).unwrap(), KeyState::Saved(Some(s)) if s=="secret-test-key")
        );
        assert!(matches!(
            read_key(&reopened, "https://different.test/v1").unwrap(),
            KeyState::Missing
        ));
        write(&mut reopened, None, Some((&profile.endpoint, ""))).unwrap();
        assert!(matches!(
            read_key(&reopened, &profile.endpoint).unwrap(),
            KeyState::Saved(None)
        ));
        let raw: String = reopened
            .query_row("SELECT data FROM profile", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Profile>(&raw).unwrap().prompt,
            profile.prompt
        );
    }

    /// The old database is retained byte-for-byte, and new keys restore from
    /// local files alone. Loss/corruption cannot create a replacement key.
    #[test]
    fn local_store_preserves_legacy_and_reopens_without_os_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("llm.db");
        fs::write(&legacy, b"unreadable legacy ciphertext").unwrap();
        let store = Store::at(dir.path());
        store
            .access(false, |db| {
                assert!(db.is_none());
                Ok(())
            })
            .unwrap();
        assert!(!store.path.exists());
        store
            .access(true, |db| {
                write(db.unwrap(), None, Some(("endpoint", "local-only-key")))
            })
            .unwrap();
        drop(store);
        let store = Store::at(dir.path());
        store.access(false, |db| {
            assert!(matches!(read_key(db.unwrap(), "endpoint")?, KeyState::Saved(Some(key)) if key == "local-only-key"));
            Ok(())
        }).unwrap();
        let path = store.path.clone();
        drop(store);
        let before = fs::read(&path).unwrap();
        let key_path = path.with_file_name("master.key");
        fs::remove_file(&key_path).unwrap();
        assert!(Store::at(dir.path()).access(true, |_| Ok(())).is_err());
        assert!(!key_path.exists());
        fs::write(&key_path, b"corrupt").unwrap();
        assert!(Store::at(dir.path()).access(true, |_| Ok(())).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&legacy).unwrap(), b"unreadable legacy ciphertext");
    }

    #[test]
    fn failed_key_write_rolls_back_profile_in_same_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = open_encrypted(&dir.path().join("llm.db"), "test-only-password").unwrap();
        db.execute_batch("CREATE TRIGGER fail_key BEFORE INSERT ON endpoint_keys BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        let profile = Profile {
            engine: "llm".into(),
            endpoint: "https://example.test/v1".into(),
            model: "test".into(),
            prompt: "test".into(),
        };
        assert!(write(
            &mut db,
            Some(&profile),
            Some((&profile.endpoint, "test-key"))
        )
        .is_err());
        assert_eq!(
            db.query_row("SELECT count(*) FROM profile", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
