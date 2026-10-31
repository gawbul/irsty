//! Opening a TCP or TLS connection to a server.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls_platform_verifier::Verifier;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{self, CryptoProvider};
use tokio_rustls::rustls::{
    self, CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, SignatureScheme,
};
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
    /// The server's certificate was rejected.
    #[error("{}", explain_certificate(.host, .error))]
    Certificate {
        /// The host we connected to.
        host: String,
        /// Why the certificate was rejected.
        error: CertificateError,
    },
}

impl ConnectError {
    /// Picks the certificate error out of a failed TLS handshake, so it can be
    /// explained rather than shown as rustls's terse `Display`.
    fn from_handshake(error: std::io::Error, host: &str) -> Self {
        match error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        {
            Some(rustls::Error::InvalidCertificate(cert)) => Self::Certificate {
                host: host.to_owned(),
                error: cert.clone(),
            },
            _ => Self::Io(error),
        }
    }
}

/// Says in plain words what is wrong with a certificate and what to do about it.
fn explain_certificate(host: &str, error: &CertificateError) -> String {
    let date = |time: &UnixTime| {
        i64::try_from(time.as_secs())
            .ok()
            .and_then(|secs| jiff::Timestamp::from_second(secs).ok())
            .map(|t| t.strftime("%Y-%m-%d").to_string())
    };
    match error {
        CertificateError::NotValidForNameContext { presented, .. }
            if presented.iter().any(|n| presented_name(n).is_some()) =>
        {
            format!(
                "the certificate is for {}, not {host}. If one of those is the same server, use it as the host",
                name_list(presented)
            )
        }
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
            format!("the certificate is not valid for {host}")
        }
        CertificateError::UnknownIssuer => "the certificate is not signed by an authority this system trusts (often it is self-signed). If it is a server you control, you can set tls_verify = false for it".to_owned(),
        CertificateError::Expired => {
            "the certificate has expired; the server's operators need to renew it".to_owned()
        }
        CertificateError::ExpiredContext { not_after, .. } => match date(not_after) {
            Some(day) => {
                format!("the certificate expired on {day}; the server's operators need to renew it")
            }
            None => explain_certificate(host, &CertificateError::Expired),
        },
        CertificateError::NotValidYet => {
            "the certificate is not valid yet; check this computer's clock".to_owned()
        }
        CertificateError::NotValidYetContext { not_before, .. } => match date(not_before) {
            Some(day) => {
                format!("the certificate is not valid until {day}; check this computer's clock")
            }
            None => explain_certificate(host, &CertificateError::NotValidYet),
        },
        CertificateError::Revoked => "the certificate has been revoked by its issuer".to_owned(),
        other => rustls::Error::InvalidCertificate(other.clone()).to_string(),
    }
}

/// Shows at most this many certificate names; some certificates carry hundreds.
const MAX_NAMES: usize = 8;

/// webpki reports names in `Debug` form, such as `DnsName("irc.example.com")`
/// and `IpAddress(192.0.2.1)`. Returns the bare name, or `None` for kinds that
/// can't be a server's host (directory names, URIs and so on).
fn presented_name(name: &str) -> Option<&str> {
    name.strip_prefix("DnsName(\"")
        .and_then(|n| n.strip_suffix("\")"))
        .or_else(|| {
            name.strip_prefix("IpAddress(")
                .and_then(|n| n.strip_suffix(')'))
        })
}

fn name_list(presented: &[String]) -> String {
    let names: Vec<&str> = presented.iter().filter_map(|n| presented_name(n)).collect();
    let mut list = names
        .iter()
        .take(MAX_NAMES)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > MAX_NAMES {
        let _ = write!(list, " and {} more", names.len() - MAX_NAMES);
    }
    list
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
        let platform = Verifier::new(Arc::clone(&provider))?;
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(ExplainingVerifier(Arc::new(platform))))
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

/// Wraps the platform verifier so its errors can be explained. macOS and
/// Windows report a bare `NotValidForName`, so we read the names the
/// certificate does cover with webpki; macOS also reports some common failures
/// only as an OS error code. It only ever refines an error; it never turns a
/// failure into success.
#[derive(Debug)]
struct ExplainingVerifier(Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for ExplainingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .map_err(|error| add_presented_names(from_apple_code(error), end_entity, server_name))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.0.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.0.root_hint_subjects()
    }
}

/// macOS's platform verifier maps only a few Security framework errors and
/// reports the rest as `Other("<description>: <code>")`. Recover the common
/// ones; codes are from `SecBase.h`.
fn from_apple_code(error: rustls::Error) -> rustls::Error {
    let rustls::Error::InvalidCertificate(CertificateError::Other(other)) = &error else {
        return error;
    };
    let text = other.to_string();
    let mapped = match text.rsplit_once(": ").map(|(_, code)| code) {
        Some("-67843") => CertificateError::UnknownIssuer, // errSecNotTrusted
        Some("-67818") => CertificateError::Expired,       // errSecCertificateExpired
        Some("-67819") => CertificateError::NotValidYet,   // errSecCertificateNotValidYet
        _ => return error,
    };
    rustls::Error::InvalidCertificate(mapped)
}

/// Turns a bare `NotValidForName` into `NotValidForNameContext`. Any other
/// error, or a certificate webpki can't read, is returned unchanged.
fn add_presented_names(
    error: rustls::Error,
    end_entity: &CertificateDer<'_>,
    server_name: &ServerName<'_>,
) -> rustls::Error {
    if !matches!(
        error,
        rustls::Error::InvalidCertificate(CertificateError::NotValidForName)
    ) {
        return error;
    }
    let Ok(cert) = webpki::EndEntityCert::try_from(end_entity) else {
        return error;
    };
    match cert.verify_is_valid_for_subject_name(server_name) {
        Err(webpki::Error::CertNotValidForName(context)) => {
            rustls::Error::InvalidCertificate(CertificateError::NotValidForNameContext {
                expected: context.expected,
                presented: context.presented,
            })
        }
        _ => error,
    }
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
        let tcp = super::dial::connect(&server.host, server.port).await?;
        tcp.set_nodelay(true)?;
        let Some(tls) = tls else {
            return Ok(Either::Left(tcp));
        };
        let name = ServerName::try_from(server.host.clone())
            .map_err(|_| ConnectError::InvalidName(server.host.clone()))?;
        let stream = tokio_rustls::TlsConnector::from(tls)
            .connect(name, tcp)
            .await
            .map_err(|e| ConnectError::from_handshake(e, &server.host))?;
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

    fn self_signed(names: &[&str]) -> rcgen::CertifiedKey<rcgen::KeyPair> {
        let names: Vec<String> = names.iter().map(|&n| n.to_owned()).collect();
        rcgen::generate_simple_self_signed(names).unwrap()
    }

    fn name(host: &str) -> ServerName<'static> {
        ServerName::try_from(host.to_owned()).unwrap()
    }

    #[test]
    fn adds_presented_names_to_a_bare_name_mismatch() {
        let cert = self_signed(&["irc.example.com", "*.example.org"]);
        let bare = || rustls::Error::InvalidCertificate(CertificateError::NotValidForName);
        let err = add_presented_names(bare(), cert.cert.der(), &name("irc.example.net"));
        let rustls::Error::InvalidCertificate(CertificateError::NotValidForNameContext {
            presented,
            ..
        }) = err
        else {
            panic!("{err:?}");
        };
        assert_eq!(name_list(&presented), "irc.example.com, *.example.org");

        // Anything else passes through untouched.
        let unknown = rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer);
        assert_eq!(
            add_presented_names(unknown.clone(), cert.cert.der(), &name("irc.example.net")),
            unknown
        );
        let garbage = CertificateDer::from(vec![1, 2, 3]);
        assert_eq!(
            add_presented_names(bare(), &garbage, &name("irc.example.net")),
            bare()
        );
        // If webpki thinks the name matches, keep the platform's verdict.
        assert_eq!(
            add_presented_names(bare(), cert.cert.der(), &name("irc.example.com")),
            bare()
        );
    }

    #[test]
    fn recovers_apple_error_codes() {
        let other = |text: &str| {
            rustls::Error::InvalidCertificate(CertificateError::Other(rustls::OtherError(
                Arc::new(std::io::Error::other(text.to_owned())),
            )))
        };
        let cert = |e| rustls::Error::InvalidCertificate(e);
        assert_eq!(
            from_apple_code(other("“x” certificate is not trusted: -67843")),
            cert(CertificateError::UnknownIssuer)
        );
        assert_eq!(
            from_apple_code(other("expired: -67818")),
            cert(CertificateError::Expired)
        );
        assert_eq!(
            from_apple_code(other("not yet: -67819")),
            cert(CertificateError::NotValidYet)
        );
        // Unknown codes and other errors are left alone.
        let unknown = from_apple_code(other("something: -1"));
        assert!(matches!(
            unknown,
            rustls::Error::InvalidCertificate(CertificateError::Other(_))
        ));
        assert_eq!(
            from_apple_code(cert(CertificateError::Revoked)),
            cert(CertificateError::Revoked)
        );
    }

    #[derive(Debug)]
    struct Fixed(Result<(), rustls::Error>);

    impl ServerCertVerifier for Fixed {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            self.0.clone().map(|()| ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Err(rustls::Error::General("unused".into()))
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Err(rustls::Error::General("unused".into()))
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ED25519]
        }
    }

    #[test]
    fn explaining_verifier_never_turns_failure_into_success() {
        let cert = self_signed(&["irc.example.com"]);
        let verify = |inner: Result<(), rustls::Error>, host: &str| {
            ExplainingVerifier(Arc::new(Fixed(inner))).verify_server_cert(
                cert.cert.der(),
                &[],
                &name(host),
                &[],
                UnixTime::now(),
            )
        };
        assert!(verify(Ok(()), "irc.example.com").is_ok());
        for error in [
            CertificateError::UnknownIssuer,
            CertificateError::Expired,
            CertificateError::NotValidForName,
        ] {
            for host in ["irc.example.com", "other.example.com"] {
                assert!(
                    verify(Err(rustls::Error::InvalidCertificate(error.clone())), host).is_err()
                );
            }
        }
        let wrapped = ExplainingVerifier(Arc::new(Fixed(Ok(()))));
        assert_eq!(
            wrapped.supported_verify_schemes(),
            [SignatureScheme::ED25519]
        );
    }

    #[test]
    fn explains_certificate_problems() {
        let explain = |e| explain_certificate("irc.example.net", &e);
        assert_eq!(
            explain(CertificateError::NotValidForNameContext {
                expected: name("irc.example.net"),
                presented: vec![
                    "DnsName(\"irc.example.com\")".into(),
                    "DirectoryName".into(),
                    "IpAddress(192.0.2.1)".into(),
                ],
            }),
            "the certificate is for irc.example.com, 192.0.2.1, not irc.example.net. \
             If one of those is the same server, use it as the host"
        );
        assert_eq!(
            explain(CertificateError::NotValidForName),
            "the certificate is not valid for irc.example.net"
        );
        // No usable names: fall back to the plain message.
        assert_eq!(
            explain(CertificateError::NotValidForNameContext {
                expected: name("irc.example.net"),
                presented: vec!["DirectoryName".into()],
            }),
            "the certificate is not valid for irc.example.net"
        );
        let many: Vec<String> = (0..20)
            .map(|i| format!("DnsName(\"h{i}.example\")"))
            .collect();
        assert!(name_list(&many).ends_with("h7.example and 12 more"));
        assert!(explain(CertificateError::UnknownIssuer).contains("tls_verify = false"));
        assert!(explain(CertificateError::Expired).contains("expired"));
        let time = UnixTime::since_unix_epoch(Duration::from_hours(500_000));
        let date = UnixTime::since_unix_epoch(Duration::from_secs(1_700_000_000));
        assert!(
            explain(CertificateError::ExpiredContext {
                time,
                not_after: date
            })
            .contains("expired on 2023-11-14")
        );
        assert!(
            explain(CertificateError::NotValidYetContext {
                time: date,
                not_before: time
            })
            .starts_with("the certificate is not valid until 2027-01-15;")
        );
        assert!(explain(CertificateError::NotValidYet).contains("clock"));
        assert!(explain(CertificateError::Revoked).contains("revoked"));
        assert!(explain(CertificateError::BadSignature).contains("invalid peer certificate"));
    }

    #[test]
    fn picks_certificate_errors_out_of_handshake_failures() {
        let tls = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(CertificateError::Revoked),
        );
        assert!(matches!(
            ConnectError::from_handshake(tls, "h"),
            ConnectError::Certificate {
                error: CertificateError::Revoked,
                ..
            }
        ));
        let other = std::io::Error::other("reset");
        assert!(matches!(
            ConnectError::from_handshake(other, "h"),
            ConnectError::Io(_)
        ));
    }

    /// End to end: a self-signed server is rejected by the real platform
    /// verifier, and the error comes back explained.
    #[tokio::test]
    async fn rejects_a_self_signed_server() {
        let cert = self_signed(&["localhost"]);
        let key = PrivateKeyDer::try_from(cert.signing_key.serialize_der()).unwrap();
        let provider = Arc::new(crypto::aws_lc_rs::default_provider());
        let server_config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.cert.der().clone()], key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let _ = acceptor.accept(tcp).await;
        });
        let server = ServerConfig {
            host: "localhost".into(),
            port,
            tls: true,
            tls_verify: true,
            password: None,
        };
        let tls = tls_config(true, None).unwrap();
        let err = connect(&server, Some(tls), Duration::from_secs(10))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                ConnectError::Certificate {
                    error: CertificateError::UnknownIssuer,
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("tls_verify = false"), "{err}");
        server_task.await.unwrap();
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
