//! TLS client configuration shared by the CacheWitness runner: Redis
//! (`rediss://`), application servers (`https://`), and PostgreSQL.
//!
//! Certificates are verified against the operating system's trust store,
//! falling back to the Mozilla roots bundled in `webpki-roots` when the system
//! store is empty (minimal containers). [`TlsOptions`] adds a private CA or
//! turns verification off for self-signed test servers.

use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

/// How to verify servers.
#[derive(Debug, Clone, Default)]
pub struct TlsOptions {
    /// Extra trusted CA certificates (PEM), e.g. a private CA for staging.
    pub ca_file: Option<PathBuf>,
    /// Do not verify server certificates at all. For self-signed test
    /// servers only.
    pub insecure: bool,
}

/// A ready-to-use TLS client configuration. Cheap to clone.
#[derive(Clone)]
pub struct TlsConfig(Arc<ClientConfig>);

impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TlsConfig")
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn builder() -> io::Result<rustls::ConfigBuilder<ClientConfig, rustls::WantsVerifier>> {
    ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)
}

impl TlsConfig {
    /// Build a configuration from `opts`.
    pub fn new(opts: &TlsOptions) -> io::Result<TlsConfig> {
        if opts.insecure {
            return TlsConfig::no_verify();
        }
        let mut roots = RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        if let Some(path) = &opts.ca_file {
            let file = File::open(path).map_err(|e| io::Error::new(e.kind(), format!("reading CA file {}: {e}", path.display())))?;
            let mut added = 0;
            for cert in rustls_pemfile_certs(BufReader::new(file))? {
                roots
                    .add(cert)
                    .map_err(|e| io::Error::other(format!("CA file {}: {e}", path.display())))?;
                added += 1;
            }
            if added == 0 {
                return Err(io::Error::other(format!("CA file {} has no PEM certificates", path.display())));
            }
        }
        Ok(TlsConfig(Arc::new(builder()?.with_root_certificates(roots).with_no_client_auth())))
    }

    /// Verify against the system trust store (or the bundled roots).
    pub fn default_roots() -> io::Result<TlsConfig> {
        TlsConfig::new(&TlsOptions::default())
    }

    /// Encrypt without verifying the server certificate. This is what libpq
    /// does for `sslmode=require`, and what [`TlsOptions::insecure`] asks for.
    pub fn no_verify() -> io::Result<TlsConfig> {
        let verifier = Arc::new(NoVerify(provider()));
        Ok(TlsConfig(Arc::new(
            builder()?
                .dangerous()
                .with_custom_certificate_verifier(verifier)
                .with_no_client_auth(),
        )))
    }

    /// The underlying rustls configuration.
    pub fn client_config(&self) -> Arc<ClientConfig> {
        self.0.clone()
    }

    /// Run a TLS handshake over `stream` for server name `host` (a DNS name or an IP address).
    pub async fn connect<S>(&self, host: &str, stream: S) -> io::Result<TlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let name = ServerName::try_from(host.to_string()).map_err(|e| io::Error::other(format!("bad TLS server name {host:?}: {e}")))?;
        TlsConnector::from(self.0.clone()).connect(name, stream).await
    }
}

/// Read every certificate from a PEM stream.
fn rustls_pemfile_certs(reader: impl io::BufRead) -> io::Result<Vec<CertificateDer<'static>>> {
    use rustls::pki_types::pem::PemObject;
    CertificateDer::pem_reader_iter(reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| io::Error::other(format!("bad PEM: {e}")))
}

/// Accepts any server certificate; still checks handshake signatures.
#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
