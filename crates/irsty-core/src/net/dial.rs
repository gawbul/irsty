//! Opening a TCP connection to a host, over IPv4 or IPv6.
//!
//! A simplified Happy Eyeballs (RFC 8305): resolve every address, alternate
//! the families starting with the resolver's first choice, and start the next
//! attempt every [`ATTEMPT_DELAY`] or as soon as one fails. The first to
//! connect wins. A host with a broken IPv6 route therefore costs a quarter of
//! a second rather than a full connect timeout.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::net::TcpStream;

/// How long to wait for an attempt before starting the next (RFC 8305 §5).
pub const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Resolves `host` and connects to the first address that answers.
pub async fn connect(host: &str, port: u16) -> io::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
    race(interleave(addrs), ATTEMPT_DELAY, TcpStream::connect).await
}

/// Alternates address families, starting with the family of the first
/// address and otherwise keeping the resolver's order.
pub(crate) fn interleave(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let Some(first) = addrs.first() else {
        return addrs;
    };
    let preferred_v6 = first.is_ipv6();
    let (mut preferred, mut other): (Vec<_>, Vec<_>) =
        addrs.into_iter().partition(|a| a.is_ipv6() == preferred_v6);
    preferred.reverse();
    other.reverse();
    let mut out = Vec::with_capacity(preferred.len() + other.len());
    loop {
        match (preferred.pop(), other.pop()) {
            (None, None) => return out,
            (a, b) => out.extend(a.into_iter().chain(b)),
        }
    }
}

/// Runs staggered connection attempts and returns the first success, or the
/// last error once every attempt has failed.
pub(crate) async fn race<T, F, Fut>(
    addrs: Vec<SocketAddr>,
    delay: Duration,
    connect: F,
) -> io::Result<T>
where
    F: Fn(SocketAddr) -> Fut,
    Fut: Future<Output = io::Result<T>>,
{
    let mut queue = addrs.into_iter();
    let mut pending = FuturesUnordered::new();
    let Some(first) = queue.next() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the host name has no addresses",
        ));
    };
    pending.push(connect(first));
    loop {
        tokio::select! {
            Some(result) = pending.next() => match result {
                Ok(stream) => return Ok(stream),
                Err(error) => match queue.next() {
                    Some(next) => pending.push(connect(next)),
                    // Every attempt failed; report the last.
                    None if pending.is_empty() => return Err(error),
                    None => {}
                },
            },
            () = tokio::time::sleep(delay) => {
                if let Some(next) = queue.next() {
                    pending.push(connect(next));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn interleaves_families_from_the_first() {
        let v6a = addr("[2001:db8::1]:6697");
        let v6b = addr("[2001:db8::2]:6697");
        let v4a = addr("192.0.2.1:6697");
        let v4b = addr("192.0.2.2:6697");
        let v4c = addr("192.0.2.3:6697");
        assert_eq!(
            interleave(vec![v6a, v6b, v4a, v4b, v4c]),
            [v6a, v4a, v6b, v4b, v4c]
        );
        assert_eq!(interleave(vec![v4a, v4b, v6a]), [v4a, v6a, v4b]);
        assert_eq!(interleave(vec![v4a]), [v4a]);
        assert!(interleave(Vec::new()).is_empty());
    }

    /// A fake connector: `hang` never answers, `fail` fails at once, anything
    /// else connects after `ms` milliseconds (its port).
    async fn fake(addr: SocketAddr) -> io::Result<SocketAddr> {
        match addr.ip().to_string().as_str() {
            "192.0.2.1" => std::future::pending().await,
            "192.0.2.2" => Err(io::Error::from(io::ErrorKind::ConnectionRefused)),
            _ => {
                tokio::time::sleep(Duration::from_millis(u64::from(addr.port()))).await;
                Ok(addr)
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_address_costs_one_delay() {
        let hang = addr("192.0.2.1:0");
        let good = addr("198.51.100.1:10");
        let start = tokio::time::Instant::now();
        assert_eq!(
            race(vec![hang, good], ATTEMPT_DELAY, fake).await.unwrap(),
            good
        );
        assert_eq!(start.elapsed(), ATTEMPT_DELAY + Duration::from_millis(10));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_starts_the_next_attempt_at_once() {
        let fail = addr("192.0.2.2:0");
        let good = addr("198.51.100.1:10");
        let start = tokio::time::Instant::now();
        assert_eq!(
            race(vec![fail, good], ATTEMPT_DELAY, fake).await.unwrap(),
            good
        );
        assert_eq!(start.elapsed(), Duration::from_millis(10));
    }

    #[tokio::test(start_paused = true)]
    async fn the_fastest_wins_and_errors_come_last() {
        // The first is slow, the second starts later but finishes sooner.
        let slow = addr("198.51.100.1:1000");
        let quick = addr("198.51.100.2:10");
        assert_eq!(
            race(vec![slow, quick], ATTEMPT_DELAY, fake).await.unwrap(),
            quick
        );
        let fail = addr("192.0.2.2:0");
        let err = race(vec![fail, fail], ATTEMPT_DELAY, fake)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
        let err = race(Vec::new(), ATTEMPT_DELAY, fake).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn connects_over_ipv6_loopback() {
        let Ok(listener) = tokio::net::TcpListener::bind("[::1]:0").await else {
            eprintln!("no IPv6 loopback here; skipping");
            return;
        };
        let port = listener.local_addr().unwrap().port();
        let stream = connect("::1", port).await.unwrap();
        assert!(stream.peer_addr().unwrap().is_ipv6());
    }
}
