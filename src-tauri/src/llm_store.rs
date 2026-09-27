//! Reinstall-stable LLM storage, independent of UI, translation and LAN identity.
//! SQLCipher encrypts ~/.translateme/llm.db; a random master key stays in the OS
//! credential store. Neither reinstall nor a missing key ever resets an old DB.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

static STORE: LazyLock<Result<Store, String>> = LazyLock::new(|| {
    let home = dirs::home_dir().ok_or("无法找到用户主目录。")?;
    Ok(Store {
        path: home.join(".translateme/llm.db"),
        connection: Mutex::new(None),
    })
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

/// A NULL row is a deliberate deletion, preventing a legacy key from reappearing.
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

/// Missing databases do not prompt for Keychain access. Creation happens only
/// on an explicit save or successful migration, never merely when testing a key.
impl Store {
    fn access<T>(
        &self,
        create: bool,
        operation: impl FnOnce(Option<&mut Connection>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut connection = self.connection.lock().map_err(|_| "LLM 存储暂不可用。")?;
        if connection.is_none() && (create || self.path.exists()) {
            let exists = self.path.exists();
            let entry = keyring::Entry::new("app.translateme.llm-storage", "sqlcipher-master-v1")
                .map_err(|_| "无法打开系统凭据库。")?;
            let existing = match entry.get_password() {
                Ok(key) => Some(key),
                Err(keyring::Error::NoEntry) => None,
                Err(_) => return Err("无法读取 LLM 数据库解密密钥，请允许系统凭据库访问。".into()),
            };
            let key = master_key(exists, existing.as_deref())?;
            if existing.is_none() {
                entry
                    .set_password(&key)
                    .map_err(|_| "无法保存数据库解密密钥，未创建数据库。")?;
                // Windows/macOS credentials must persist before writing ciphertext.
                if entry.get_password().ok().as_deref() != Some(&key) {
                    return Err("系统凭据库未能持久保存解密密钥，未创建数据库。".into());
                }
            }
            *connection = Some(open_encrypted(&self.path, &key)?);
        }
        operation(connection.as_mut())
    }
}

/// A surviving DB without its matching OS credential is recoverable ciphertext,
/// not an empty installation. Never generate a replacement key in that case.
fn master_key(database_exists: bool, saved: Option<&str>) -> Result<String, String> {
    match saved {
        Some(key) if !key.is_empty() => Ok(key.into()),
        // An empty-but-existing OS item is corrupt state, not a newly generated
        // key. Refuse it even before DB creation rather than lose the next open.
        Some(_) => Err("系统凭据中的 LLM 解密密钥为空，未创建或改动数据库。".into()),
        None if database_exists => Err(
            "LLM 数据库仍在，但本机解密密钥不可用。请恢复原系统账户的凭据；原文件未改动。".into(),
        ),
        _ => Ok(rand::random::<[u8; 32]>()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()),
    }
}

/// Set the key before touching schema, verify the linked library is SQLCipher,
/// then authenticate existing pages. A wrong key/corrupt file must never migrate.
fn open_encrypted(path: &Path, key: &str) -> Result<Connection, String> {
    let parent = path.parent().ok_or("LLM 数据库目录不可用。")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建 ~/.translateme 目录。")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法保护数据库目录。")?;
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|_| "无法打开 LLM 数据库文件。")?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "无法保护数据库文件。")?;
    }
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

/// Preserve the distinction between missing (may migrate) and deleted (must not).
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

    /// Tests use an isolated random credential, never the user's real Keychain.
    #[test]
    fn encrypted_file_reopens_and_wrong_key_does_not_modify_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("llm.db");
        let key = master_key(false, None).unwrap();
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
        // Simulate reinstall: all in-memory state is gone; original file + OS key survive.
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

    #[test]
    fn lost_master_key_never_generates_a_replacement_for_existing_data() {
        assert!(master_key(true, None).is_err());
        assert!(master_key(true, Some("")).is_err());
        assert!(master_key(false, Some("")).is_err());
        assert_eq!(master_key(true, Some("existing")).unwrap(), "existing");
        assert_ne!(
            master_key(false, None).unwrap(),
            master_key(false, None).unwrap()
        );
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
