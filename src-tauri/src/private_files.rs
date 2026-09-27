//! Small local-file boundary for LLM keys and the LAN private identity. Files
//! are protected before secret bytes are written; no OS credential API is used.
//! This protects against other ordinary accounts, not software running as this
//! user or someone who obtains a complete backup of the data directory.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

/// Create an application-owned directory and restrict it to this account.
/// Symlink destinations are rejected so permission changes stay on our files.
pub fn directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| "无法创建本地密钥目录。")?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path).map_err(|_| "无法创建本地密钥目录。")?;
    protect(path)?;
    if !path.is_dir() {
        return Err("本地密钥目录被同名文件占用。".into());
    }
    Ok(())
}

/// Apply owner-only permissions to an existing regular file or directory.
/// Refuse unsupported file types instead of following links to unrelated data.
pub fn protect(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法检查本地密钥文件。")?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("本地密钥路径不能使用链接或特殊文件。".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if metadata.is_dir() { 0o700 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|_| "无法限制本地密钥文件权限。")?;
    }
    #[cfg(windows)]
    windows_acl::protect(path, metadata.is_dir())?;
    Ok(())
}

/// Bounded reads distinguish absence from corrupt/unreadable state. Callers
/// must never replace an existing database/identity after one of these errors.
pub fn read(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法检查本地密钥文件。".into()),
        Ok(metadata) if !metadata.is_file() => return Err("本地密钥必须是普通文件。".into()),
        Ok(_) => {}
    }
    protect(path)?;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "无法打开本地密钥文件。")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取本地密钥文件。")?;
    if bytes.len() > limit {
        return Err("本地密钥文件格式异常，原文件未改动。".into());
    }
    Ok(Some(bytes))
}

/// Publish a complete secret once, without overwriting another instance's
/// winner. False means the caller should read/validate the existing file.
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    let parent = path.parent().ok_or("本地密钥目录不可用。")?;
    directory(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法准备本地密钥文件。")?;
    protect(temp.path())?;
    temp.write_all(bytes)
        .and_then(|_| temp.flush())
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|_| "无法保存本地密钥文件。")?;
    match temp.persist_noclobber(path) {
        Ok(_) => {
            sync_directory(parent)?;
            Ok(true)
        }
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) => Err("无法完成本地密钥保存。".into()),
    }
}

/// Flush a published directory entry on Unix before a database can commit data
/// encrypted by its new key. Windows file publication uses flushed file data.
pub fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| "无法完成本地密钥目录保存。")?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(windows)]
mod windows_acl {
    use std::{os::windows::ffi::OsStrExt, path::Path};
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL},
            Security::{
                Authorization::{
                    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                    SDDL_REVISION_1,
                },
                GetTokenInformation, SetFileSecurityW, TokenUser, DACL_SECURITY_INFORMATION,
                PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };

    /// Handles/LocalAlloc buffers must be released on every intermediate error.
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    struct Local(HLOCAL);
    impl Drop for Local {
        fn drop(&mut self) {
            unsafe {
                let _ = LocalFree(Some(self.0));
            }
        }
    }

    /// Replace inherited grants with a protected DACL for the current token's
    /// user SID. Directory ACEs also protect newly created descendants. This
    /// is a filesystem ACL only: no Credential Manager or DPAPI access occurs.
    pub fn protect(path: &Path, directory: bool) -> Result<(), String> {
        unsafe { apply(path, directory) }
            .map_err(|_| "无法限制本地密钥文件权限，请使用支持账户权限的本机目录。".into())
    }

    /// Token data is stored in pointer-aligned memory; SID pointers stay valid
    /// through SDDL construction. Secret contents never enter this function.
    unsafe fn apply(path: &Path, directory: bool) -> windows::core::Result<()> {
        let mut raw = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw)?;
        let token = Token(raw);
        let mut needed = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut needed);
        if needed == 0 || needed > 16384 {
            return Err(windows::core::Error::from_win32());
        }
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut sid)?;
        let _sid = Local(HLOCAL(sid.0.cast()));
        let inheritance = if directory { "OICI" } else { "" };
        let sddl: Vec<u16> = format!("D:P(A;{inheritance};FA;;;{})", sid.to_string()?)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?;
        let _descriptor = Local(HLOCAL(descriptor.0));
        let filename: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        SetFileSecurityW(
            PCWSTR(filename.as_ptr()),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Existing data must survive an oversized file or an attempted replacement;
    /// private modes are established before any secret bytes are published.
    #[test]
    fn publication_never_replaces_existing_secret() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private/key");
        assert!(write_new(&path, b"original").unwrap());
        assert!(!write_new(&path, b"replacement").unwrap());
        assert_eq!(read(&path, 64).unwrap().unwrap(), b"original");
        assert!(read(&path, 2).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_never_followed_or_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("unrelated");
        let link = temp.path().join("key");
        fs::write(&target, b"original").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read(&link, 64).is_err());
        assert!(!write_new(&link, b"replacement").unwrap());
        assert_eq!(fs::read(target).unwrap(), b"original");
    }
}
