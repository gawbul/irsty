//! A SOCKS5 client (RFC 1928), CONNECT only, with optional username and
//! password authentication (RFC 1929).
//!
//! Host names are sent to the proxy unresolved, so DNS goes through the proxy
//! too. That matters for Tor, where a local lookup would reveal where you are
//! connecting.

use std::net::IpAddr;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const VERSION: u8 = 5;
const NO_AUTH: u8 = 0x00;
const USER_PASS: u8 = 0x02;
const NO_ACCEPTABLE: u8 = 0xff;
const CONNECT: u8 = 0x01;
const ATYP_V4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_V6: u8 = 0x04;

/// Why the proxy would not connect us.
#[derive(Debug, thiserror::Error)]
pub enum SocksError {
    /// Reading from or writing to the proxy failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The proxy said something that isn't SOCKS5.
    #[error("the proxy does not speak SOCKS5")]
    Protocol,
    /// The proxy wants authentication we can't offer.
    #[error("the proxy accepts none of our authentication methods")]
    NoAcceptableAuth,
    /// The proxy rejected the username or password.
    #[error("the proxy rejected the username or password")]
    AuthFailed,
    /// A host name, username or password is longer than SOCKS5 allows.
    #[error("{0} is longer than SOCKS5 allows (255 bytes)")]
    TooLong(&'static str),
    /// The proxy could not reach the server.
    #[error("the proxy could not connect: {}", reply_text(*.0))]
    Refused(u8),
}

fn reply_text(code: u8) -> &'static str {
    match code {
        1 => "general failure",
        2 => "not allowed by its rules",
        3 => "network unreachable",
        4 => "host unreachable",
        5 => "connection refused",
        6 => "timed out",
        7 => "command not supported",
        8 => "address type not supported",
        _ => "unknown error",
    }
}

fn short(field: &'static str, value: &[u8]) -> Result<u8, SocksError> {
    u8::try_from(value.len()).map_err(|_| SocksError::TooLong(field))
}

/// Asks the proxy on `stream` to connect to `host:port`. On success the stream
/// carries the connection to the server.
pub async fn handshake<S>(
    stream: &mut S,
    host: &str,
    port: u16,
    credentials: Option<(&str, &str)>,
) -> Result<(), SocksError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Validate everything before sending anything.
    let host_bytes = host.as_bytes();
    let ip: Option<IpAddr> = host.parse().ok();
    if ip.is_none() {
        short("the host name", host_bytes)?;
    }
    if let Some((user, pass)) = credentials {
        short("the proxy username", user.as_bytes())?;
        short("the proxy password", pass.as_bytes())?;
    }

    let greeting: &[u8] = if credentials.is_some() {
        &[VERSION, 2, NO_AUTH, USER_PASS]
    } else {
        &[VERSION, 1, NO_AUTH]
    };
    stream.write_all(greeting).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    match choice {
        [VERSION, NO_AUTH] => {}
        [VERSION, USER_PASS] => {
            let Some((user, pass)) = credentials else {
                return Err(SocksError::NoAcceptableAuth);
            };
            let mut auth = vec![1, short("the proxy username", user.as_bytes())?];
            auth.extend_from_slice(user.as_bytes());
            auth.push(short("the proxy password", pass.as_bytes())?);
            auth.extend_from_slice(pass.as_bytes());
            stream.write_all(&auth).await?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await?;
            if status[1] != 0 {
                return Err(SocksError::AuthFailed);
            }
        }
        [VERSION, NO_ACCEPTABLE] => return Err(SocksError::NoAcceptableAuth),
        _ => return Err(SocksError::Protocol),
    }

    let mut request = vec![VERSION, CONNECT, 0];
    match ip {
        Some(IpAddr::V4(v4)) => {
            request.push(ATYP_V4);
            request.extend_from_slice(&v4.octets());
        }
        Some(IpAddr::V6(v6)) => {
            request.push(ATYP_V6);
            request.extend_from_slice(&v6.octets());
        }
        None => {
            request.extend_from_slice(&[ATYP_DOMAIN, short("the host name", host_bytes)?]);
            request.extend_from_slice(host_bytes);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    let [version, code, _reserved, atyp] = reply;
    if version != VERSION {
        return Err(SocksError::Protocol);
    }
    if code != 0 {
        return Err(SocksError::Refused(code));
    }
    // Skip the address the proxy bound, which we don't need.
    let skip = match atyp {
        ATYP_V4 => 4 + 2,
        ATYP_V6 => 16 + 2,
        ATYP_DOMAIN => usize::from(stream.read_u8().await?) + 2,
        _ => return Err(SocksError::Protocol),
    };
    let mut bound = vec![0u8; skip];
    stream.read_exact(&mut bound).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{DuplexStream, duplex};

    /// Runs `handshake` against a scripted proxy: `expect` is what the client
    /// must send at each step and `reply` what the proxy answers.
    async fn run(
        host: &str,
        credentials: Option<(&str, &str)>,
        script: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), SocksError> {
        let (mut client, mut proxy): (DuplexStream, DuplexStream) = duplex(1024);
        let proxy_task = tokio::spawn(async move {
            for (expect, reply) in script {
                let mut got = vec![0u8; expect.len()];
                if proxy.read_exact(&mut got).await.is_err() {
                    return;
                }
                assert_eq!(got, expect);
                proxy.write_all(&reply).await.unwrap();
            }
        });
        let result = handshake(&mut client, host, 6697, credentials).await;
        drop(client);
        proxy_task.await.unwrap();
        result
    }

    const OK_V4: [u8; 10] = [5, 0, 0, 1, 127, 0, 0, 1, 0x1a, 0x29];

    #[tokio::test]
    async fn connects_by_name_without_resolving_locally() {
        let mut request = vec![5, 1, 0, 3, 15];
        request.extend_from_slice(b"irc.libera.chat");
        request.extend_from_slice(&6697u16.to_be_bytes());
        run(
            "irc.libera.chat",
            None,
            vec![(vec![5, 1, 0], vec![5, 0]), (request, OK_V4.to_vec())],
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sends_ip_literals_as_addresses() {
        let mut v4 = vec![5, 1, 0, 1, 192, 0, 2, 1];
        v4.extend_from_slice(&6697u16.to_be_bytes());
        run(
            "192.0.2.1",
            None,
            vec![(vec![5, 1, 0], vec![5, 0]), (v4, OK_V4.to_vec())],
        )
        .await
        .unwrap();

        let mut v6 = vec![5, 1, 0, 4];
        v6.extend_from_slice(
            &"2001:db8::1"
                .parse::<std::net::Ipv6Addr>()
                .unwrap()
                .octets(),
        );
        v6.extend_from_slice(&6697u16.to_be_bytes());
        // A reply bound to a domain name is skipped correctly too.
        let reply = [&[5u8, 0, 0, 3, 3][..], b"abc", &[0, 1]].concat();
        run(
            "2001:db8::1",
            None,
            vec![(vec![5, 1, 0], vec![5, 0]), (v6, reply)],
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn authenticates_with_username_and_password() {
        let mut request = vec![5, 1, 0, 1, 192, 0, 2, 1];
        request.extend_from_slice(&6697u16.to_be_bytes());
        run(
            "192.0.2.1",
            Some(("me", "pw")),
            vec![
                (vec![5, 2, 0, 2], vec![5, 2]),
                (vec![1, 2, b'm', b'e', 2, b'p', b'w'], vec![1, 0]),
                (request, OK_V4.to_vec()),
            ],
        )
        .await
        .unwrap();

        let err = run(
            "192.0.2.1",
            Some(("me", "wrong")),
            vec![
                (vec![5, 2, 0, 2], vec![5, 2]),
                (
                    vec![1, 2, b'm', b'e', 5, b'w', b'r', b'o', b'n', b'g'],
                    vec![1, 1],
                ),
            ],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SocksError::AuthFailed), "{err}");
    }

    #[tokio::test]
    async fn reports_proxy_failures() {
        let refuse = |code| {
            let mut request = vec![5, 1, 0, 1, 192, 0, 2, 1];
            request.extend_from_slice(&6697u16.to_be_bytes());
            vec![(vec![5, 1, 0], vec![5, 0]), (request, vec![5, code, 0, 1])]
        };
        let err = run("192.0.2.1", None, refuse(5)).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "the proxy could not connect: connection refused"
        );
        let err = run("192.0.2.1", None, refuse(99)).await.unwrap_err();
        assert!(err.to_string().ends_with("unknown error"));

        // Needs a password we don't have.
        let err = run("h", None, vec![(vec![5, 1, 0], vec![5, 2])])
            .await
            .unwrap_err();
        assert!(matches!(err, SocksError::NoAcceptableAuth));
        let err = run("h", None, vec![(vec![5, 1, 0], vec![5, 0xff])])
            .await
            .unwrap_err();
        assert!(matches!(err, SocksError::NoAcceptableAuth));
        // Not SOCKS5 at all, for example an HTTP proxy.
        let err = run("h", None, vec![(vec![5, 1, 0], b"HT".to_vec())])
            .await
            .unwrap_err();
        assert!(matches!(err, SocksError::Protocol));
        // The proxy hangs up.
        let err = run("h", None, Vec::new()).await.unwrap_err();
        assert!(matches!(err, SocksError::Io(_)));
    }

    #[tokio::test]
    async fn rejects_overlong_fields_before_sending() {
        let long = "a".repeat(256);
        let err = run(&long, None, Vec::new()).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "the host name is longer than SOCKS5 allows (255 bytes)"
        );
        let err = run("h", Some((&long, "p")), Vec::new()).await.unwrap_err();
        assert!(matches!(err, SocksError::TooLong("the proxy username")));
    }
}
