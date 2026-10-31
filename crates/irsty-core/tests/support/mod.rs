//! The scripted mock server and client helpers shared by the integration
//! tests. Each test binary uses a different subset, hence the allowance.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions, shared between test binaries"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use irsty_core::client::SettingsHook;
use irsty_core::state::Status;
use irsty_core::{Client, Config};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, timeout};

pub const STEP: Duration = Duration::from_secs(5);

pub struct Mock {
    pub listener: TcpListener,
}

impl Mock {
    pub async fn bind() -> Self {
        Self {
            listener: TcpListener::bind("127.0.0.1:0").await.unwrap(),
        }
    }

    pub fn port(&self) -> u16 {
        self.listener.local_addr().unwrap().port()
    }

    pub async fn accept(&self) -> Conn {
        let (stream, _) = timeout(STEP, self.listener.accept())
            .await
            .expect("client did not connect")
            .unwrap();
        Conn::new(stream)
    }
}

pub struct Conn {
    pub lines: Lines<BufReader<OwnedReadHalf>>,
    pub writer: OwnedWriteHalf,
}

impl Conn {
    pub fn new(stream: TcpStream) -> Self {
        let (read, writer) = stream.into_split();
        Self {
            lines: BufReader::new(read).lines(),
            writer,
        }
    }

    pub async fn recv(&mut self) -> String {
        timeout(STEP, self.lines.next_line())
            .await
            .expect("timed out waiting for the client")
            .unwrap()
            .expect("client closed the connection")
    }

    pub async fn expect(&mut self, expected: &str) {
        let line = self.recv().await;
        assert_eq!(line, expected);
    }

    pub async fn send(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .unwrap();
    }

    /// Registration without CAP support beyond what is passed.
    pub async fn register(&mut self, nick: &str, caps: &str) {
        self.expect("CAP LS 302").await;
        self.expect(&format!("NICK {nick}")).await;
        self.expect(&format!("USER {nick} 0 * {nick}")).await;
        self.send(&format!(":srv CAP * LS :{caps}")).await;
        if caps.is_empty() {
            self.expect("CAP END").await;
        } else {
            let req = self.recv().await;
            assert!(req.starts_with("CAP REQ"), "{req}");
            let list = req
                .trim_start_matches("CAP REQ ")
                .trim_start_matches(':')
                .to_owned();
            self.send(&format!(":srv CAP * ACK :{list}")).await;
            self.expect("CAP END").await;
        }
        self.send(&format!(":srv 001 {nick} :Welcome")).await;
    }

    /// Waits until the client closes the connection.
    pub async fn closed(&mut self) {
        loop {
            match timeout(STEP, self.lines.next_line())
                .await
                .expect("connection stayed open")
            {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return,
            }
        }
    }
}

pub fn config(port: u16, extra: &str, log_dir: &Path) -> Config {
    let text = format!(
        r##"
        [logging]
        dir = "{}"

        [[network]]
        name = "mock"
        nicks = ["me", "me_"]
        autojoin = ["#test"]
        {extra}

        [[network.servers]]
        host = "127.0.0.1"
        port = {port}
        tls = false
        "##,
        log_dir.display()
    );
    Config::parse(&text, Path::new("test.toml")).unwrap()
}

pub fn fast(ping_interval: Duration) -> SettingsHook {
    Arc::new(move |s| {
        s.timeouts.registration = Duration::from_secs(5);
        s.timeouts.ping_interval = ping_interval;
        s.timeouts.ping_timeout = Duration::from_millis(300);
        s.rate.burst = 100;
        s.rate.interval = Duration::from_millis(10);
        s.backoff_base = Duration::from_millis(50);
        s.backoff_cap = Duration::from_millis(100);
    })
}

/// Drives the client until `done` holds. Polls on a short tick so conditions
/// that do not depend on client events (a finished mock) are noticed;
/// `Client::next` is cancel-safe.
pub async fn until(client: &mut Client, mut done: impl FnMut(&Client) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(client) {
        assert!(Instant::now() < deadline, "condition never held");
        let _ = timeout(Duration::from_millis(20), client.next()).await;
    }
}

/// Drives the client until the mock server's script has finished. The client
/// must keep running, as effects such as rejoining come from the app loop.
pub async fn drive<T>(client: &mut Client, server: tokio::task::JoinHandle<T>) -> T {
    until(client, |_| server.is_finished()).await;
    server.await.unwrap()
}

pub fn server_text(client: &Client) -> Vec<String> {
    let state = client.state();
    let net = state.networks().next().unwrap();
    state
        .buffer(net.server_buffer)
        .unwrap()
        .lines()
        .iter()
        .map(|l| l.text.to_plain())
        .collect()
}

pub fn status(client: &Client) -> Status {
    client.state().networks().next().unwrap().status
}
