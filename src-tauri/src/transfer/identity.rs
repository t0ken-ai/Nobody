//! Each installation has a stable, self-issued TLS certificate. TLS proves key
//! possession; the transfer handshake separately gates all data on saved trust
//! or an explicitly compared TLS-exporter code. Web/LLM TLS is never affected.
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Credential-store payload only; never serialize this into app files or UI events.
#[derive(Serialize, Deserialize)]
struct SavedIdentity {
    cert: Vec<u8>,
    key: Vec<u8>,
}
/// Stable installation credential; only its public fingerprint leaves this module.
pub struct Identity {
    pub id: String,
    saved: SavedIdentity,
}
/// SHA-256 of the full stable certificate is the wire identity, never an IP or
/// advertised name. A replacement certificate always needs fresh consent.
pub fn fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
impl Identity {
    /// The OS credential entry is separate from endpoint-scoped LLM credentials.
    /// Failure does not create an unpersisted identity that changes next launch.
    pub fn load() -> Result<Self, String> {
        let entry = keyring::Entry::new("app.translateme.desktop.lan", "device-identity-v1")
            .map_err(|e| e.to_string())?;
        let saved = match entry.get_secret() {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|_| "设备身份损坏，请检查系统凭据库。")?
            }
            Err(keyring::Error::NoEntry) => {
                let value = Self::generate()?.saved;
                entry
                    .set_secret(&serde_json::to_vec(&value).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("无法保存设备身份：{e}"))?;
                value
            }
            Err(e) => return Err(format!("无法读取设备身份：{e}")),
        };
        let result = Self {
            id: fingerprint(&saved.cert),
            saved,
        };
        // Verify stored certificate/key consistency before exposing the service.
        result.server_config()?;
        Ok(result)
    }
    /// Also used by isolated two-peer tests; tests never read real credentials.
    pub fn generate() -> Result<Self, String> {
        let certified = rcgen::generate_simple_self_signed(vec!["translateme.local".into()])
            .map_err(|e| e.to_string())?;
        let saved = SavedIdentity {
            cert: certified.cert.der().to_vec(),
            key: certified.signing_key.serialize_der(),
        };
        Ok(Self {
            id: fingerprint(&saved.cert),
            saved,
        })
    }
    /// Give rustls its own PKCS#8 buffer without exposing private bytes to callers.
    fn key(&self) -> PrivateKeyDer<'static> {
        PrivatePkcs8KeyDer::from(self.saved.key.clone()).into()
    }
    /// TLS 1.3 and mandatory client certificates bind both endpoints. Unknown
    /// certificates can only reach the bounded application pairing handshake.
    pub fn server_config(&self) -> Result<Arc<ServerConfig>, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .with_client_cert_verifier(Arc::new(PeerVerifier { expected: None }))
            .with_single_cert(
                vec![CertificateDer::from(self.saved.cert.clone())],
                self.key(),
            )
            .map_err(|e| e.to_string())?;
        config.alpn_protocols = vec![b"translateme/1".to_vec()];
        config.max_early_data_size = 0;
        Ok(Arc::new(config))
    }
    /// Discovery supplies a candidate fingerprint; known peers are selected by
    /// their saved fingerprint. Never accept a different certificate at that IP.
    pub fn client_config(&self, expected: String) -> Result<Arc<ClientConfig>, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PeerVerifier {
                expected: Some(expected),
            }))
            .with_client_auth_cert(
                vec![CertificateDer::from(self.saved.cert.clone())],
                self.key(),
            )
            .map_err(|e| e.to_string())?;
        config.alpn_protocols = vec![b"translateme/1".to_vec()];
        config.enable_early_data = false;
        Ok(Arc::new(config))
    }
}

/// This is certificate pinning with an application-level first-use ceremony,
/// not public-CA validation. Signature checks below remain rustls' real crypto
/// checks; returning a fabricated handshake-signature success is never allowed.
#[derive(Debug)]
struct PeerVerifier {
    expected: Option<String>,
}
impl PeerVerifier {
    /// Accept a bounded leaf certificate and enforce the selected peer pin when supplied.
    fn check(
        &self,
        cert: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
    ) -> Result<(), rustls::Error> {
        if cert.len() > 4096 || cert.is_empty() || !chain.is_empty() {
            return Err(rustls::Error::General("invalid device certificate".into()));
        }
        if self
            .expected
            .as_ref()
            .is_some_and(|pin| pin != &fingerprint(cert))
        {
            return Err(rustls::Error::General("device identity changed".into()));
        }
        Ok(())
    }
    /// Verify possession of the certificate key using rustls' TLS 1.3 algorithms.
    fn signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    /// Advertise exactly the signature schemes implemented by the selected provider.
    fn schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
// Outgoing connections pin the selected identity; CA names/expiry are not the
// trust model. Both verifier traits still require a valid TLS handshake signature.
impl ServerCertVerifier for PeerVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.check(cert, chain)?;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 disabled".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.signature(m, c, s)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes()
    }
}
// Incoming unknown identities can reach pairing, never payload before consent.
impl ClientCertVerifier for PeerVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        cert: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.check(cert, chain)?;
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 disabled".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.signature(m, c, s)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes()
    }
}
