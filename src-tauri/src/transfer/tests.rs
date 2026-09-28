//! Real loopback TLS tests use isolated identities/directories, never the user's
//! home directory or inbox. They exercise the full trust gate and streaming protocol.
use super::*;
use protocol::{read_frame, write_frame, FileMeta, Frame};
use std::{fs, net::SocketAddr};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{TlsConnector, TlsStream};

/// Updates may neither interrupt an existing job nor race a fresh reservation;
/// a failed installer releases its pause without disabling the user's service.
#[tokio::test]
async fn update_pause_preserves_jobs_and_recovers_after_failure() {
    let f = fixture("update-guard").await;
    let (id, _) = f.service.begin("peer".into(), "Peer".into(), false).unwrap();
    assert!(f.service.pause_for_update().is_err());
    f.service.inner.lock().unwrap().active.remove(&id);
    let pause = f.service.pause_for_update().unwrap();
    assert!(f.service.begin("peer".into(), "Peer".into(), true).is_err());
    assert!(f.service.retry().await.is_err());
    drop(pause);
    assert!(f.service.begin("peer".into(), "Peer".into(), false).is_ok());
    assert!(f.service.inner.lock().unwrap().settings.enabled);
}

struct Fixture {
    service: Service,
    identity: Arc<Identity>,
    addr: SocketAddr,
    stop: CancellationToken,
    root: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.cancel();
        self.service.stop();
    }
}
/// Launch production listener with an ephemeral identity on loopback only.
async fn fixture(name: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let service = TransferService::new(
        root.path().join("config"),
        root.path().join("received"),
        Arc::new(|_| {}),
    )
    .unwrap();
    let identity = Arc::new(Identity::load(&service.store.dir).unwrap());
    *service.identity.lock().unwrap() = Some(identity.clone());
    {
        let mut i = service.inner.lock().unwrap();
        i.status = "running".into();
        i.device_id = identity.id.clone();
        i.settings.name = name.into();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    tokio::spawn(transport::listen(
        service.clone(),
        identity.clone(),
        listener,
        stop.clone(),
    ));
    Fixture {
        service,
        identity,
        addr,
        stop,
        root,
    }
}
fn advertise(a: &Fixture, b: &Fixture) {
    a.service.inner.lock().unwrap().peers.insert(
        b.identity.id.clone(),
        Peer {
            id: b.identity.id.clone(),
            name: b.service.inner.lock().unwrap().settings.name.clone(),
            platform: "macOS".into(),
            online: true,
            trusted: false,
            addresses: vec![b.addr],
            service_name: "fixture".into(),
        },
    );
}
fn trust(a: &Fixture, b: &Fixture) {
    a.service
        .trust(TrustedPeer {
            id: b.identity.id.clone(),
            name: "fixture".into(),
            platform: "macOS".into(),
        })
        .unwrap();
}
/// Tests fail promptly instead of hanging when protocol transitions regress.
async fn until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("protocol state transition timed out");
}
fn terminal(s: &Service, id: &str) -> bool {
    s.inner
        .lock()
        .unwrap()
        .records
        .iter()
        .any(|r| r.id == id && ["completed", "failed", "cancelled"].contains(&r.phase.as_str()))
}

/// Frequent progress snapshots must stay small without damaging the full text
/// retained for explicit copy/view. Use multibyte input to catch byte slicing.
#[test]
fn history_preview_does_not_copy_or_truncate_the_stored_body() {
    let record = Record {
        id: "preview".into(),
        peer_id: "peer".into(),
        peer_name: "Mac".into(),
        direction: "received".into(),
        phase: "completed".into(),
        created_at: 1,
        bytes: 0,
        total: 0,
        text: "中🙂".repeat(100_000),
        files: vec![],
        paths: vec![PathBuf::from("/received/example.txt")],
        error: String::new(),
    };
    let preview = record.preview();
    assert_eq!(preview.text, "中🙂".repeat(90));
    assert!(
        preview.text.capacity() < 4096,
        "a preview must not retain a full-body allocation"
    );
    assert_eq!(record.text.len(), 700_000);
    assert_eq!(preview.paths, record.paths);
    assert_eq!(preview.id, record.id);
}

#[tokio::test]
async fn first_pairing_streams_text_files_and_preserves_existing_names() {
    let a = fixture("A").await;
    let b = fixture("B").await;
    advertise(&a, &b);
    let data = a.root.path().join("源码.txt");
    let empty = a.root.path().join("empty.bin");
    fs::write(&data, "let count = 42;\n").unwrap();
    fs::write(&empty, []).unwrap();
    // Span several 64 KiB chunks, including non-UTF-8 bytes and a short tail.
    let binary = a.root.path().join("chunks.bin");
    let bytes: Vec<u8> = (0..(256 * 1024 + 19)).map(|n| (n % 256) as u8).collect();
    fs::write(&binary, &bytes).unwrap();
    let folder = b.service.inner.lock().unwrap().settings.receive_dir.clone();
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("源码.txt"), "original").unwrap();
    let job = a
        .service
        .send(
            b.identity.id.clone(),
            "多行文字\nDo not translate `git diff`.".into(),
            vec![data, empty, binary],
        )
        .unwrap();
    until(|| a.service.snapshot().pairings.len() == 1 && b.service.snapshot().pairings.len() == 1)
        .await;
    let pa = a.service.snapshot().pairings.remove(0);
    let pb = b.service.snapshot().pairings.remove(0);
    assert_eq!(
        pa.code, pb.code,
        "TLS exporter code must match at both endpoints"
    );
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        1,
        "no payload before consent"
    );
    a.service.decide(&pa.id, true).unwrap();
    b.service.decide(&pb.id, true).unwrap();
    until(|| terminal(&a.service, &job)).await;
    assert_eq!(a.service.snapshot().records[0].phase, "completed");
    assert!(a.service.is_trusted(&b.identity.id) && b.service.is_trusted(&a.identity.id));
    let received = b.service.inner.lock().unwrap().records[0].clone();
    assert_eq!(received.text, "多行文字\nDo not translate `git diff`.");
    assert_eq!(received.paths.len(), 3);
    assert_eq!(fs::read(&received.paths[2]).unwrap(), bytes);
    assert_eq!(
        fs::read_to_string(folder.join("源码.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        fs::read_to_string(&received.paths[0]).unwrap(),
        "let count = 42;\n"
    );
    assert_eq!(fs::metadata(&received.paths[1]).unwrap().len(), 0);
    assert!(received.paths[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("(1)"));
    let second = a
        .service
        .send(b.identity.id.clone(), "already trusted".into(), vec![])
        .unwrap();
    until(|| terminal(&a.service, &second)).await;
    assert!(a.service.snapshot().pairings.is_empty() && b.service.snapshot().pairings.is_empty());
    assert_eq!(a.service.snapshot().records[0].phase, "completed");
    b.service.clear().unwrap();
    assert!(
        received.paths[0].exists(),
        "clearing records must not delete files"
    );
    assert_eq!(
        b.service
            .store
            .read::<Vec<TrustedPeer>>("device/trusted.json")
            .unwrap()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn rejecting_pairing_writes_no_payload_and_dismisses_other_prompt() {
    let a = fixture("A").await;
    let b = fixture("B").await;
    advertise(&a, &b);
    let job = a
        .service
        .send(b.identity.id.clone(), "must not arrive".into(), vec![])
        .unwrap();
    until(|| !b.service.snapshot().pairings.is_empty()).await;
    b.service
        .decide(&b.service.snapshot().pairings[0].id, false)
        .unwrap();
    until(|| terminal(&a.service, &job) && a.service.snapshot().pairings.is_empty()).await;
    assert_eq!(a.service.snapshot().records[0].phase, "failed");
    assert!(!a.service.is_trusted(&b.identity.id));
    assert!(!b.service.is_trusted(&a.identity.id));
    assert!(b
        .service
        .inner
        .lock()
        .unwrap()
        .records
        .iter()
        .all(|r| r.text.is_empty() && r.paths.is_empty()));
}

#[tokio::test]
async fn wrong_certificate_cannot_inherit_selected_device_identity() {
    let a = fixture("A").await;
    let b = fixture("B").await;
    advertise(&a, &b);
    let fake = Identity::generate().unwrap();
    let mut peer = a
        .service
        .inner
        .lock()
        .unwrap()
        .peers
        .remove(&b.identity.id)
        .unwrap();
    peer.id = fake.id.clone();
    a.service
        .inner
        .lock()
        .unwrap()
        .peers
        .insert(fake.id.clone(), peer);
    let job = a.service.send(fake.id, "secret".into(), vec![]).unwrap();
    until(|| terminal(&a.service, &job)).await;
    assert_eq!(a.service.snapshot().records[0].phase, "failed");
    assert!(b.service.snapshot().pairings.is_empty());
}

/// Connect a controlled peer through real TLS to test malformed/partial streams.
async fn raw(a: &Fixture, b: &Fixture, hello: bool) -> TlsStream<TcpStream> {
    let tcp = TcpStream::connect(b.addr).await.unwrap();
    let tls = TlsConnector::from(a.identity.client_config(b.identity.id.clone()).unwrap())
        .connect(
            rustls::pki_types::ServerName::try_from("translateme.local").unwrap(),
            tcp,
        )
        .await
        .unwrap();
    let mut stream: TlsStream<_> = tls.into();
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        Frame::Hello { .. }
    ));
    if hello {
        write_frame(
            &mut stream,
            &Frame::Hello {
                version: 1,
                id: a.identity.id.clone(),
                name: "test peer".into(),
                platform: "macOS".into(),
                trusted: true,
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            Frame::Ready
        ));
        write_frame(&mut stream, &Frame::Ready).await.unwrap();
    }
    stream
}
#[tokio::test]
async fn unknown_peer_cannot_skip_pairing_with_an_offer() {
    let a = fixture("A").await;
    let b = fixture("B").await;
    let mut stream = raw(&a, &b, false).await;
    write_frame(
        &mut stream,
        &Frame::Offer {
            text_size: 0,
            files: vec![FileMeta {
                name: "secret.txt".into(),
                size: 1,
            }],
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        Frame::Error { .. }
    ));
    assert!(!b
        .service
        .inner
        .lock()
        .unwrap()
        .settings
        .receive_dir
        .exists());
}
#[tokio::test]
async fn disconnect_and_bad_digest_clean_partial_files() {
    let a = fixture("A").await;
    let b = fixture("B").await;
    trust(&a, &b);
    trust(&b, &a);
    let folder = b.service.inner.lock().unwrap().settings.receive_dir.clone();
    for corrupt in [false, true] {
        let before = b.service.snapshot().records.len();
        let mut stream = raw(&a, &b, true).await;
        write_frame(
            &mut stream,
            &Frame::Offer {
                text_size: 0,
                files: vec![FileMeta {
                    name: "partial.txt".into(),
                    size: 3,
                }],
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            Frame::Accept
        ));
        stream
            .write_all(if corrupt { b"abc" } else { b"a" })
            .await
            .unwrap();
        stream.flush().await.unwrap();
        if corrupt {
            write_frame(
                &mut stream,
                &Frame::Digest {
                    sha256: "incorrect".into(),
                },
            )
            .await
            .unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                Frame::Error { .. }
            ));
        }
        drop(stream);
        until(|| {
            b.service.snapshot().records.len() > before
                && b.service.snapshot().records[0].phase == "failed"
        })
        .await;
        assert_eq!(
            fs::read_dir(&folder).unwrap().count(),
            0,
            "no orphan data on failure"
        );
    }
}
#[tokio::test]
async fn revoking_trust_cancels_existing_stream() {
    let a = fixture("A").await;
    let b = fixture("B").await;
    trust(&a, &b);
    trust(&b, &a);
    let mut stream = raw(&a, &b, true).await;
    write_frame(
        &mut stream,
        &Frame::Offer {
            text_size: 0,
            files: vec![FileMeta {
                name: "cancel.bin".into(),
                size: 1024,
            }],
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        Frame::Accept
    ));
    stream.write_all(b"partial").await.unwrap();
    stream.flush().await.unwrap();
    b.service.forget(&a.identity.id).unwrap();
    until(|| b.service.snapshot().records[0].phase == "cancelled").await;
    assert!(!b.service.is_trusted(&a.identity.id));
    let folder = b.service.inner.lock().unwrap().settings.receive_dir.clone();
    assert_eq!(fs::read_dir(folder).unwrap().count(), 0);
}
#[tokio::test]
async fn cancelling_pairing_removes_pending_prompts() {
    let a = fixture("A").await;
    let b = fixture("B").await;
    advertise(&a, &b);
    let job = a
        .service
        .send(b.identity.id.clone(), "cancel".into(), vec![])
        .unwrap();
    until(|| !a.service.snapshot().pairings.is_empty()).await;
    a.service.cancel(&job).unwrap();
    until(|| terminal(&a.service, &job) && b.service.snapshot().pairings.is_empty()).await;
    assert_eq!(a.service.snapshot().records[0].phase, "cancelled");
}
#[test]
fn filenames_limits_and_directories_are_rejected_before_connecting() {
    for name in [
        "../x", "C:\\x", "a/b", "x:ads", "CON.txt", "lpt1", "..", "", "space ", "trail.", "a\nb",
    ] {
        assert!(!protocol::valid_name(name), "{name}");
    }
    assert!(protocol::valid_name("源码-hello.ts"));
    assert!(protocol::validate_offer(protocol::MAX_TEXT + 1, &[]).is_err());
    assert!(protocol::validate_offer(
        0,
        &[FileMeta {
            name: "ok".into(),
            size: protocol::MAX_FILE + 1
        }]
    )
    .is_err());
    let folder = tempfile::tempdir().unwrap();
    assert!(transport::inspect_files(&[folder.path().to_path_buf()]).is_err());
}
#[tokio::test]
async fn oversized_control_frame_is_rejected_without_allocating_payload() {
    let (mut writer, mut reader) = tokio::io::duplex(32);
    writer.write_u32(u32::MAX).await.unwrap();
    assert!(read_frame(&mut reader).await.is_err());
}
/// Actual multicast is opt-in: CI loopback TLS tests must not pretend they prove
/// discovery through a user's router/firewall. Run explicitly on the test Mac.
#[tokio::test]
#[ignore]
async fn local_multicast_discovery() {
    let a = fixture("TranslateMe test A").await;
    let b = fixture("TranslateMe test B").await;
    a.service.start().await;
    b.service.start().await;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if a.service
                .snapshot()
                .peers
                .iter()
                .any(|p| p.id == b.identity.id)
                && b.service
                    .snapshot()
                    .peers
                    .iter()
                    .any(|p| p.id == a.identity.id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    a.service.stop();
    b.service.stop();
    assert!(
        result.is_ok(),
        "multicast discovery did not find both isolated peers"
    );
}

#[test]
fn crash_cleanup_only_removes_this_installations_staging() {
    let folder = tempfile::tempdir().unwrap();
    let own = folder
        .path()
        .join(format!("{}abc", store::staging_prefix("own")));
    let other = folder
        .path()
        .join(format!("{}abc", store::staging_prefix("other")));
    fs::create_dir(&own).unwrap();
    fs::write(own.join("partial"), b"partial").unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(folder.path().join("keep.txt"), b"keep").unwrap();
    store::cleanup_staging(folder.path(), "own").unwrap();
    assert!(!own.exists());
    assert!(other.exists());
    assert!(folder.path().join("keep.txt").exists());
}

/// Files may be published only if their receipt can be saved. Simulate both a
/// destination collision and a failed inbox replacement without filling disk.
#[tokio::test]
async fn destination_and_receipt_write_failures_never_report_completion() {
    for fail_receipt in [false, true] {
        let a = fixture("A").await;
        let b = fixture("B").await;
        advertise(&a, &b);
        trust(&a, &b);
        trust(&b, &a);
        let folder = b.service.inner.lock().unwrap().settings.receive_dir.clone();
        if fail_receipt {
            fs::create_dir(b.service.store.dir.join("inbox.json")).unwrap();
        } else {
            fs::create_dir_all(folder.parent().unwrap()).unwrap();
            fs::write(&folder, b"existing file must survive").unwrap();
        }
        let source = a.root.path().join("sample.txt");
        fs::write(&source, b"test file").unwrap();
        let job = a
            .service
            .send(b.identity.id.clone(), "test text".into(), vec![source])
            .unwrap();
        until(|| terminal(&a.service, &job)).await;
        until(|| {
            b.service
                .snapshot()
                .records
                .first()
                .is_some_and(|r| r.phase == "failed")
        })
        .await;
        assert_eq!(a.service.snapshot().records[0].phase, "failed");
        assert!(b.service.snapshot().records[0].text.is_empty());
        if fail_receipt {
            assert_eq!(
                fs::read_dir(folder).unwrap().count(),
                0,
                "published files roll back if receipt fails"
            );
        } else {
            assert_eq!(fs::read(folder).unwrap(), b"existing file must survive");
        }
    }
}

/// Failure from a cancelled listener cannot overwrite a replacement service's
/// state; an actual listener failure must make the disconnected state visible.
#[tokio::test]
async fn listener_failure_is_visible_and_stale_failure_is_ignored() {
    let a = fixture("A").await;
    let stale = CancellationToken::new();
    stale.cancel();
    a.service.network_failed(&stale, "stale".into()).await;
    assert_eq!(a.service.snapshot().status, "running");
    a.service
        .network_failed(&a.stop, "test listener failure".into())
        .await;
    assert_eq!(a.service.snapshot().status, "error");
    assert_eq!(a.service.snapshot().error, "test listener failure");
}

/// Opt-in native UI diagnostic: publishes one ephemeral peer, then requires an
/// exact session id/code in approval.json after the operator compares the GUI.
/// Uses no real keychain; only synthetic reply files live in a temporary folder.
/// status.json exposes public pairing/job data; stop.txt or ten minutes ends it.
/// send-again.txt requests another synthetic reply to the same paired UI, so
/// background receiving can be checked after the app window is closed.
#[tokio::test]
#[ignore]
async fn native_ui_diagnostic_peer() {
    let control =
        PathBuf::from(std::env::var("TRANSLATEME_QA_DIR").expect("set a dedicated QA directory"));
    assert!(control.is_absolute() && control.is_dir());
    let peer = fixture("TranslateMe · 本机测试").await;
    peer.service.start().await;
    let binary = peer.root.path().join("互传检查.bin");
    fs::write(
        &binary,
        (0..(256 * 1024 + 19))
            .map(|n| (n % 256) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let mut replied = false;
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline && !control.join("stop.txt").exists() {
        let snapshot = peer.service.snapshot();
        fs::write(
            control.join("status.json"),
            serde_json::to_vec_pretty(&snapshot).unwrap(),
        )
        .unwrap();
        if let Ok(bytes) = fs::read(control.join("approval.json")) {
            let approval: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if let Some(p) = snapshot
                .pairings
                .iter()
                .find(|p| approval["id"] == p.id && approval["code"] == p.code)
            {
                peer.service.decide(&p.id, true).unwrap();
            }
            fs::remove_file(control.join("approval.json")).unwrap();
        }
        // Reply once to the explicitly paired GUI, only after real receipt and
        // discovery of its address; no other discovered device is contacted.
        if !replied || control.join("send-again.txt").exists() {
            if let Some(record) = snapshot
                .records
                .iter()
                .find(|r| r.direction == "received" && r.phase == "completed")
            {
                if snapshot
                    .peers
                    .iter()
                    .any(|p| p.id == record.peer_id && p.online && p.trusted)
                {
                    peer.service.send(record.peer_id.clone(), "本机测试设备已收到。\nKeep the current data visible.\ngit diff --check".into(), vec![binary.clone()]).unwrap();
                    replied = true;
                    let _ = fs::remove_file(control.join("send-again.txt"));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    peer.service.stop();
}

/// UI adapters may run outside Tokio. Reject that misuse before reserving a job
/// instead of panicking (the native Send button previously hit this boundary).
#[test]
fn send_without_runtime_returns_an_error_and_leaves_no_job() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (a, b) = runtime.block_on(async { (fixture("A").await, fixture("B").await) });
    advertise(&a, &b);
    assert!(a
        .service
        .send(b.identity.id.clone(), "runtime check".into(), vec![])
        .is_err());
    assert!(a.service.snapshot().records.is_empty());
    assert!(a.service.inner.lock().unwrap().active.is_empty());
}

/// Upgrading without OS credentials preserves history/settings but never grants
/// a newly generated local identity the old installation's saved trust.
#[test]
fn local_identity_upgrade_leaves_old_trust_and_history_untouched() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("transfer")).unwrap();
    store
        .write(
            "trusted.json",
            &[TrustedPeer {
                id: "old-peer".into(),
                name: "Other Mac".into(),
                platform: "macOS".into(),
            }],
        )
        .unwrap();
    store.write("inbox.json", &Vec::<Record>::new()).unwrap();
    let before = fs::read(store.dir.join("trusted.json")).unwrap();
    let history = fs::read(store.dir.join("inbox.json")).unwrap();
    let service = TransferService::new(
        store.dir.clone(),
        root.path().join("received"),
        Arc::new(|_| {}),
    )
    .unwrap();
    assert!(!service.is_trusted("old-peer"));
    let identity = Identity::load(&store.dir).unwrap();
    service
        .trust(TrustedPeer {
            id: "new-peer".into(),
            name: "Confirmed Mac".into(),
            platform: "macOS".into(),
        })
        .unwrap();
    drop(service);
    let restored = TransferService::new(
        store.dir.clone(),
        root.path().join("received"),
        Arc::new(|_| {}),
    )
    .unwrap();
    assert!(restored.is_trusted("new-peer"));
    assert!(!restored.is_trusted("old-peer"));
    assert_eq!(Identity::load(&store.dir).unwrap().id, identity.id);
    assert_eq!(fs::read(store.dir.join("trusted.json")).unwrap(), before);
    assert_eq!(fs::read(store.dir.join("inbox.json")).unwrap(), history);
}
