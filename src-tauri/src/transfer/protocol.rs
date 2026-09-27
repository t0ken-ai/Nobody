//! Versioned, bounded LAN protocol. JSON carries control metadata; text/files
//! follow as length-delimited bytes so file size never dictates memory usage.
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

// v1 caps bound metadata, memory and disk exposure; file data uses 64 KiB chunks.
pub const VERSION: u16 = 1;
pub const MAX_TEXT: u64 = 1024 * 1024;
pub const MAX_FILES: usize = 100;
pub const MAX_FILE: u64 = 20 * 1024 * 1024 * 1024;
pub const MAX_TOTAL: u64 = 40 * 1024 * 1024 * 1024;
pub const IO_TIMEOUT: Duration = Duration::from_secs(30);
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_FRAME: usize = 64 * 1024;

/// Offer metadata contains a leaf name and byte count, never a destination path.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMeta {
    pub name: String,
    pub size: u64,
}
/// Trust handshake precedes Offer/Accept, payload, per-file Digest and Complete.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Frame {
    Hello {
        version: u16,
        id: String,
        name: String,
        platform: String,
        trusted: bool,
    },
    Consent {
        accepted: bool,
    },
    Ready,
    Offer {
        text_size: u64,
        files: Vec<FileMeta>,
    },
    Accept,
    Digest {
        sha256: String,
    },
    Complete,
    Error {
        message: String,
    },
}

/// Bound even control traffic before allocating. A stalled peer cannot occupy
/// a transfer slot forever; the outer task also observes cancellation.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Frame, String> {
    tokio::time::timeout(IO_TIMEOUT, async {
        let size = r.read_u32().await.map_err(io_error)? as usize;
        if size == 0 || size > MAX_FRAME {
            return Err("对方发送了无效的数据帧。".into());
        }
        let mut bytes = vec![0; size];
        r.read_exact(&mut bytes).await.map_err(io_error)?;
        serde_json::from_slice(&bytes).map_err(|_| "对方协议格式不正确。".into())
    })
    .await
    .map_err(|_| "等待对方响应超时。".to_string())?
}
/// Serialize one bounded control frame and flush it within the ordinary IO deadline.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &Frame) -> Result<(), String> {
    let bytes = serde_json::to_vec(frame).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FRAME {
        return Err("传输元数据过大。".into());
    }
    tokio::time::timeout(IO_TIMEOUT, async {
        w.write_u32(bytes.len() as u32).await.map_err(io_error)?;
        w.write_all(&bytes).await.map_err(io_error)?;
        w.flush().await.map_err(io_error)
    })
    .await
    .map_err(|_| "发送超时。".to_string())?
}
/// Preserve local diagnostics; transport sanitizes these before replying to a peer.
pub fn io_error(e: std::io::Error) -> String {
    format!("连接或文件读写失败：{e}")
}

/// Use the same filename rules on both OSes: remote paths, Windows device
/// names/ADS, separators and trailing dots must never escape the receive folder.
pub fn valid_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 200
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    ![
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&stem.as_str())
}
/// Validate the entire offer before allocating files or accepting payload bytes.
pub fn validate_offer(text_size: u64, files: &[FileMeta]) -> Result<u64, String> {
    if text_size > MAX_TEXT || files.len() > MAX_FILES || (text_size == 0 && files.is_empty()) {
        return Err("每次最多发送 1 MiB 文字和 100 个文件。".into());
    }
    let mut total = text_size;
    for f in files {
        if !valid_name(&f.name) {
            return Err(format!("文件名无法跨平台接收：{}", f.name));
        }
        if f.size > MAX_FILE {
            return Err("单个文件不能超过 20 GiB。".into());
        }
        total = total.checked_add(f.size).ok_or("传输大小无效。")?;
    }
    if total > MAX_TOTAL {
        return Err("单次传输不能超过 40 GiB。".into());
    }
    Ok(total)
}
