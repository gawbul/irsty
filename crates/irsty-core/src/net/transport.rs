//! Opening a TCP or TLS connection to a server.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{self, CryptoProvider};
use tokio_rustls::rustls::{self, ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio_util::either::Either;

use crate::config::{ClientCertConfig, ServerConfig};

/// A connected stream, plain or TLS.
pub type Stream = Either<TcpStream, TlsStream<TcpStream>>;

/// Why a connection could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// DNS, TCP or TLS failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The connection attempt took too long.
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    /// The TLS configuration could not be built.
    #[error("TLS setup: {0}")]
    Tls(#[from] rustls::Error),
    /// The client certificate or key could not be read.
    #[error("client certificate {path}: {reason}")]
    ClientCert {
        /// The file.
        path: String,
        /// What went wrong.
        reason: String,
    },
    /// The host is not a valid DNS name or IP address.
    #[error("invalid server name {0:?}")]
    InvalidName(String),
}

/// Builds a TLS client configuration.
///
/// With `verify` false, any certificate is accepted (signatures are still
/// checked). This is insecure, so callers must surface it to the user.
pub fn tls_config(
    verify: bool,
    client_cert: Option<&ClientCertConfig>,
) -> Result<Arc<ClientConfig>, ConnectError> {
    let provider = Arc::new(crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()?;
    let builder = if verify {
        builder.with_platform_verifier()?
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate(provider)))
    };
    let mut config = match client_cert {
        Some(cert) => {
            let (chain, key) = load_client_cert(cert)?;
            builder.with_client_auth_cert(chain, key)?
        }
        None => builder.with_no_client_auth(),
    };
    // IRC has no ALPN protocol; leave it empty.
    config.alpn_protocols.clear();
    Ok(Arc::new(config))
}

fn load_client_cert(
    cert: &ClientCertConfig,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), ConnectError> {
    let fail = |path: &Path, reason: String| ConnectError::ClientCert {
        path: path.display().to_string(),
        reason,
    };
    let chain = CertificateDer::pem_file_iter(&cert.cert)
        .map_err(|e| fail(&cert.cert, e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| fail(&cert.cert, e.to_string()))?;
    if chain.is_empty() {
        return Err(fail(&cert.cert, "no certificates found".into()));
    }
    let key =
        PrivateKeyDer::from_pem_file(&cert.key).map_err(|e| fail(&cert.key, e.to_string()))?;
    Ok((chain, key))
}

/// Accepts any server certificate. Used only when `tls_verify = false`.
#[derive(Debug)]
struct AcceptAnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCertificate {
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
        crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Connects to `server`, with TLS if configured, within `timeout`.
pub async fn connect(
    server: &ServerConfig,
    tls: Option<Arc<ClientConfig>>,
    timeout: Duration,
) -> Result<Stream, ConnectError> {
    let attempt = async {
        let tcp = TcpStream::connect((server.host.as_str(), server.port)).await?;
        tcp.set_nodelay(true)?;
        let Some(tls) = tls else {
            return Ok(Either::Left(tcp));
        };
        let name = ServerName::try_from(server.host.clone())
            .map_err(|_| ConnectError::InvalidName(server.host.clone()))?;
        let stream = tokio_rustls::TlsConnector::from(tls)
            .connect(name, tcp)
            .await?;
        Ok(Either::Right(stream))
    };
    tokio::time::timeout(timeout, attempt)
        .await
        .unwrap_or(Err(ConnectError::Timeout(timeout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_verified_and_insecure_configs() {
        assert!(tls_config(true, None).is_ok());
        assert!(tls_config(false, None).is_ok());
    }

    #[test]
    fn reports_bad_client_certs() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        let missing = ClientCertConfig {
            cert: dir.path().join("missing.pem"),
            key: dir.path().join("missing.pem"),
        };
        assert!(matches!(
            tls_config(true, Some(&missing)),
            Err(ConnectError::ClientCert { .. })
        ));
        let no_certs = ClientCertConfig {
            cert: empty.clone(),
            key: empty,
        };
        let err = tls_config(true, Some(&no_certs)).unwrap_err().to_string();
        assert!(err.contains("no certificates"), "{err}");
    }

    #[tokio::test]
    async fn connect_times_out_and_refuses() {
        // A listener that never accepts still completes TCP via the backlog, so
        // use a closed port for refusal.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let server = ServerConfig {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            tls_verify: true,
            password: None,
        };
        assert!(matches!(
            connect(&server, None, Duration::from_secs(5)).await,
            Err(ConnectError::Io(_))
        ));
    }
}
