//! TLS authenticated streams and bounded file IO. Only this module handles raw
//! payload bytes; UI events contain progress, never file bodies or TLS secrets.
use super::{
    bounded_name,
    identity::fingerprint,
    platform_name,
    protocol::*,
    store::{publish_file, TrustedPeer},
    Identity, Peer, Service,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};
use tokio_util::sync::CancellationToken;
type Stream = TlsStream<TcpStream>;

/// Reject directories, symlinks, devices and platform-unsafe names before any
/// connection or pairing prompt. Recheck the opened handle again during send.
pub fn inspect_files(paths: &[PathBuf]) -> Result<Vec<FileMeta>, String> {
    if paths.len() > MAX_FILES {
        return Err("每次最多选择 100 个文件。".into());
    }
    paths
        .iter()
        .map(|path| {
            if !path.is_absolute() {
                return Err("请选择本机文件。".into());
            }
            let meta = std::fs::symlink_metadata(path).map_err(io_error)?;
            if !meta.is_file() {
                return Err("只支持普通文件，不能发送文件夹或符号链接。".into());
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("文件名不是有效文字。")?
                .to_string();
            if !valid_name(&name) {
                return Err(format!("文件名无法跨平台接收：{name}"));
            }
            if meta.len() > MAX_FILE {
                return Err("单个文件不能超过 20 GiB。".into());
            }
            Ok(FileMeta {
                name,
                size: meta.len(),
            })
        })
        .collect()
}
/// Cap unauthenticated sockets as well as active jobs. A slow/malicious TLS
/// handshake gets ten seconds and cannot create an unlimited task backlog.
pub async fn listen(
    service: Service,
    identity: Arc<Identity>,
    listener: TcpListener,
    stop: CancellationToken,
) {
    let config = match identity.server_config() {
        Ok(c) => c,
        Err(e) => {
            service.network_failed(&stop, e).await;
            return;
        }
    };
    let acceptor = TlsAcceptor::from(config);
    loop {
        tokio::select! {
            _=stop.cancelled()=>break,
            accepted=listener.accept()=>{
                let (tcp,address)=match accepted {
                    Ok(connection)=>connection,
                    Err(e)=>{service.network_failed(&stop,format!("局域网监听中断：{e}")).await;break;}
                };
                if !super::discovery::local_address(address.ip()){continue;}
                let Ok(permit)=service.slots.clone().try_acquire_owned()else{continue;};
                let s=service.clone();let identity=identity.clone();let acceptor=acceptor.clone();let stop=stop.clone();
                tokio::spawn(async move{
                    let _permit=permit;
                    let accepted=tokio::select!{_ = stop.cancelled()=>return,r=tokio::time::timeout(Duration::from_secs(10),acceptor.accept(tcp))=>r};
                    let Ok(Ok(tls))=accepted else{return;};
                    let mut stream:Stream=tls.into();
                    let peer_id=match peer_fingerprint(&stream){Ok(id)=>id,Err(_)=>return};
                    if peer_id==identity.id{return;}
                    let Ok((job,cancel))=s.begin(peer_id,"连接中的设备".into(),true)else{return;};
                    let outcome=tokio::select!{
                        _=stop.cancelled()=>Err("局域网互传已关闭。".into()),
                        _=cancel.cancelled()=>Err("传输已取消。".into()),
                        r=async{
                            let peer=authenticate(&s,&identity,&job,&mut stream,true).await?;
                            receive(&s,&job,&peer,&mut stream).await
                        }=>r
                    };
                    if let Err(message)=&outcome{let _=tokio::time::timeout(Duration::from_secs(1),write_frame(&mut stream,&Frame::Error{message:public_error(message)})).await;}
                    s.finish(&job,outcome);
                });
            }
        }
    }
}
/// Do not send OS paths or underlying key-store errors to the other machine.
fn public_error(message: &str) -> String {
    if message.contains("拒绝") || message.contains("取消") {
        "接收方拒绝或取消了传输。".into()
    } else {
        "接收方未能完成传输，请在接收端查看原因。".into()
    }
}
/// Require the LAN ALPN before reading the mutually authenticated leaf identity.
fn peer_fingerprint(stream: &Stream) -> Result<String, String> {
    let state = stream.get_ref().1;
    if state.alpn_protocol() != Some(b"translateme/1".as_slice()) {
        return Err("局域网协议版本不匹配。".into());
    }
    state
        .peer_certificates()
        .and_then(|c| c.first())
        .map(|c| fingerprint(c))
        .ok_or("对方未提供设备身份。".into())
}
/// The displayed code comes from TLS exporter material, not broadcast data or
/// a hash chosen by the peer. Both endpoints must explicitly compare it once.
fn pairing_code(stream: &Stream) -> Result<String, String> {
    let label = b"EXPORTER-TranslateMe-pair-v1";
    let bytes = match stream {
        TlsStream::Client(s) => s.get_ref().1.export_keying_material([0u8; 32], label, None),
        TlsStream::Server(s) => s.get_ref().1.export_keying_material([0u8; 32], label, None),
    }
    .map_err(|e| e.to_string())?;
    let number = u64::from_be_bytes(bytes[..8].try_into().unwrap()) % 1_000_000;
    Ok(format!("{number:06}"))
}
/// Even mutual TLS with an unknown self-issued certificate proves possession,
/// not trust. No Offer or payload is processed until both Ready messages arrive.
async fn authenticate(
    service: &Service,
    identity: &Identity,
    job: &str,
    stream: &mut Stream,
    incoming: bool,
) -> Result<TrustedPeer, String> {
    let peer_id = peer_fingerprint(stream)?;
    let known = service.is_trusted(&peer_id);
    let name = service.inner.lock().unwrap().settings.name.clone();
    write_frame(
        stream,
        &Frame::Hello {
            version: VERSION,
            id: identity.id.clone(),
            name,
            platform: platform_name(),
            trusted: known,
        },
    )
    .await?;
    let Frame::Hello {
        version,
        id,
        name,
        platform,
        trusted,
    } = read_frame(stream).await?
    else {
        return Err("对方未发送设备握手。".into());
    };
    if version != VERSION || id != peer_id || id == identity.id {
        return Err("设备身份或协议版本不匹配。".into());
    }
    let peer = TrustedPeer {
        id,
        name: bounded_name(&name)?,
        platform: bounded_name(&platform)?,
    };
    service.update(job, |r| {
        r.peer_name = peer.name.clone();
        r.peer_id = peer.id.clone();
        r.phase = "pairing".into();
    });
    if !(known && trusted) {
        let code = pairing_code(stream)?;
        // Split allows an immediate remote rejection to dismiss the local
        // prompt, rather than waiting for the entire two-minute consent timer.
        let (mut reader, mut writer) = tokio::io::split(&mut *stream);
        let local = async {
            let accepted = service.consent(job, &peer, code, incoming).await?;
            write_frame(&mut writer, &Frame::Consent { accepted }).await?;
            if accepted {
                Ok(())
            } else {
                Err("已拒绝信任此设备。".to_string())
            }
        };
        let remote = async {
            // Pairing has its own longer deadline; ordinary frames still have
            // short IO timeouts, so wait for the first byte before read_frame.
            let mut first = [0u8; 1];
            tokio::time::timeout(PAIR_TIMEOUT, reader.read_exact(&mut first))
                .await
                .map_err(|_| "对方确认信任超时。".to_string())?
                .map_err(io_error)?;
            let mut prefix = std::io::Cursor::new(first);
            let mut joined = tokio::io::AsyncReadExt::chain(&mut prefix, &mut reader);
            match read_frame(&mut joined).await? {
                Frame::Consent { accepted: true } => Ok(()),
                _ => Err("对方拒绝或结束了配对。".to_string()),
            }
        };
        tokio::try_join!(local, remote)?;
        service.trust(peer.clone())?;
    }
    write_frame(stream, &Frame::Ready).await?;
    if !matches!(read_frame(stream).await?, Frame::Ready) {
        return Err("对方尚未准备接收。".into());
    }
    if !service.is_trusted(&peer.id) {
        return Err("设备信任已撤销。".into());
    }
    Ok(peer)
}
/// Connect only to discovery-owned addresses, then authenticate against the
/// selected identity. A stale address cannot silently retarget a send.
pub async fn send(
    service: Service,
    identity: Arc<Identity>,
    job: &str,
    peer: Peer,
    text: String,
    paths: Vec<PathBuf>,
    files: Vec<FileMeta>,
) -> Result<(), String> {
    let mut tcp = None;
    for address in peer.addresses.iter().take(4) {
        if let Ok(Ok(s)) =
            tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(address)).await
        {
            tcp = Some(s);
            break;
        }
    }
    let tcp = tcp.ok_or("无法连接目标电脑，请确认它在线且局域网／防火墙允许连接。")?;
    tcp.set_nodelay(true).map_err(io_error)?;
    let connector = TlsConnector::from(identity.client_config(peer.id)?);
    let tls = tokio::time::timeout(
        Duration::from_secs(10),
        connector.connect(
            rustls::pki_types::ServerName::try_from("translateme.local").unwrap(),
            tcp,
        ),
    )
    .await
    .map_err(|_| "设备加密连接超时。".to_string())?
    .map_err(|e| format!("设备身份验证失败：{e}"))?;
    let mut stream: Stream = tls.into();
    let verified = authenticate(&service, &identity, job, &mut stream, false).await?;
    if !service.is_trusted(&verified.id) {
        return Err("设备信任已撤销。".into());
    }
    write_frame(
        &mut stream,
        &Frame::Offer {
            text_size: text.len() as u64,
            files: files.clone(),
        },
    )
    .await?;
    match read_frame(&mut stream).await? {
        Frame::Accept => {}
        Frame::Error { message } => return Err(message),
        _ => return Err("对方未接受传输。".into()),
    }
    service.update(job, |r| r.phase = "transferring".into());
    let mut transferred = 0;
    send_bytes(&mut stream, text.as_bytes()).await?;
    transferred += text.len() as u64;
    service.progress(job, transferred);
    for (path, meta) in paths.iter().zip(&files) {
        let checked = inspect_files(std::slice::from_ref(path))?;
        if checked[0].size != meta.size {
            return Err("发送前文件发生变化，请重新选择。".into());
        }
        let mut file = tokio::fs::File::open(path).await.map_err(io_error)?;
        let opened = file.metadata().await.map_err(io_error)?;
        if !opened.is_file() || opened.len() != meta.size {
            return Err("文件发生变化，请重新选择。".into());
        }
        let mut remaining = meta.size;
        let mut buffer = vec![0; 64 * 1024];
        let mut hash = Sha256::new();
        while remaining > 0 {
            let n = remaining.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..n]).await.map_err(io_error)?;
            send_bytes(&mut stream, &buffer[..n]).await?;
            hash.update(&buffer[..n]);
            remaining -= n as u64;
            transferred += n as u64;
            service.progress(job, transferred);
        }
        let mut extra = [0u8; 1];
        let after = file.metadata().await.map_err(io_error)?;
        if file.read(&mut extra).await.map_err(io_error)? != 0
            || after.len() != opened.len()
            || after.modified().ok() != opened.modified().ok()
        {
            return Err("发送期间文件发生变化，请重试。".into());
        }
        let sha256 = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        write_frame(&mut stream, &Frame::Digest { sha256 }).await?;
    }
    service.update(job, |r| r.phase = "verifying".into());
    match read_frame(&mut stream).await? {
        Frame::Complete => Ok(()),
        Frame::Error { message } => Err(message),
        _ => Err("对方未确认完整接收。".into()),
    }
}
/// Apply a deadline per IO operation instead of per file so large files on a
/// slow but progressing LAN can finish without unlimited stalled connections.
async fn send_bytes(stream: &mut Stream, bytes: &[u8]) -> Result<(), String> {
    tokio::time::timeout(IO_TIMEOUT, stream.write_all(bytes))
        .await
        .map_err(|_| "发送停滞，连接超时。".to_string())?
        .map_err(io_error)
}
/// Bound a stalled raw payload read; cancellation is additionally handled by the job.
async fn receive_bytes(stream: &mut Stream, bytes: &mut [u8]) -> Result<(), String> {
    tokio::time::timeout(IO_TIMEOUT, stream.read_exact(bytes))
        .await
        .map_err(|_| "接收停滞，连接超时。".to_string())?
        .map(|_| ())
        .map_err(io_error)
}

/// Published files remain rollback-owned until the inbox is durably updated.
/// No pre-existing file is ever included in this guard's cleanup list.
struct Published {
    paths: Vec<PathBuf>,
    committed: bool,
}
impl Drop for Published {
    fn drop(&mut self) {
        if !self.committed {
            for p in &self.paths {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}
/// Validate after trust, stage and hash every file, then commit the batch and receipt
/// before acknowledging success. RAII rolls back uncommitted paths on all exits.
async fn receive(
    service: &Service,
    job: &str,
    peer: &TrustedPeer,
    stream: &mut Stream,
) -> Result<(), String> {
    let Frame::Offer { text_size, files } = read_frame(stream).await? else {
        return Err("对方没有发送传输清单。".into());
    };
    let total = validate_offer(text_size, &files)?;
    if !service.is_trusted(&peer.id) {
        return Err("设备信任已撤销。".into());
    }
    let folder = service.inner.lock().unwrap().settings.receive_dir.clone();
    if !files.is_empty() {
        tokio::fs::create_dir_all(&folder).await.map_err(io_error)?;
    }
    // Stage on the destination volume for atomic no-clobber publishing. TempDir
    // removes all uncommitted data when the task is cancelled or disconnected.
    let staging = if files.is_empty() {
        None
    } else {
        Some(
            tempfile::Builder::new()
                .prefix(&super::store::staging_prefix(
                    &service.inner.lock().unwrap().device_id,
                ))
                .tempdir_in(&folder)
                .map_err(io_error)?,
        )
    };
    service.update(job, |r| {
        r.files = files.clone();
        r.total = total;
        r.phase = "transferring".into();
    });
    write_frame(stream, &Frame::Accept).await?;
    let mut text = vec![0; text_size as usize];
    receive_bytes(stream, &mut text).await?;
    let text = String::from_utf8(text).map_err(|_| "收到的文字不是有效 UTF-8。")?;
    let mut transferred = text_size;
    let mut completed = Vec::new();
    for meta in &files {
        let temp =
            tempfile::NamedTempFile::new_in(staging.as_ref().unwrap().path()).map_err(io_error)?;
        let mut writer = tokio::fs::File::from_std(temp.reopen().map_err(io_error)?);
        let mut hash = Sha256::new();
        let mut remaining = meta.size;
        let mut buffer = vec![0; 64 * 1024];
        while remaining > 0 {
            let n = remaining.min(buffer.len() as u64) as usize;
            receive_bytes(stream, &mut buffer[..n]).await?;
            writer.write_all(&buffer[..n]).await.map_err(io_error)?;
            hash.update(&buffer[..n]);
            remaining -= n as u64;
            transferred += n as u64;
            service.progress(job, transferred);
        }
        writer.flush().await.map_err(io_error)?;
        writer.sync_all().await.map_err(io_error)?;
        drop(writer);
        let expected = hash
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if !matches!(read_frame(stream).await?,Frame::Digest{sha256} if sha256==expected) {
            return Err("文件完整性校验失败，请重新发送。".into());
        }
        completed.push((temp, meta.name.clone()));
    }
    service.update(job, |r| r.phase = "verifying".into());
    if !service.is_trusted(&peer.id) {
        return Err("设备信任已撤销。".into());
    }
    let mut published = Published {
        paths: Vec::new(),
        committed: false,
    };
    for (temp, name) in completed {
        published.paths.push(publish_file(temp, &folder, &name)?);
    }
    service.received(job, text, published.paths.clone())?;
    published.committed = true;
    // A lost acknowledgment cannot undo a durable local receipt. The sender
    // reports missing confirmation, while the receiver retains the valid data.
    let _ = write_frame(stream, &Frame::Complete).await;
    Ok(())
}
