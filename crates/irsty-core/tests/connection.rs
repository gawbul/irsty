//! End-to-end tests of the connection task and client against a scripted mock
//! server over real sockets.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use irsty_core::client::SettingsHook;
use irsty_core::state::{LineKind, Status};
use irsty_core::{Client, Config};
use irsty_proto::Message;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, timeout};

const STEP: Duration = Duration::from_secs(5);

struct Mock {
    listener: TcpListener,
}

impl Mock {
    async fn bind() -> Self {
        Self {
            listener: TcpListener::bind("127.0.0.1:0").await.unwrap(),
        }
    }

    fn port(&self) -> u16 {
        self.listener.local_addr().unwrap().port()
    }

    async fn accept(&self) -> Conn {
        let (stream, _) = timeout(STEP, self.listener.accept())
            .await
            .expect("client did not connect")
            .unwrap();
        Conn::new(stream)
    }
}

struct Conn {
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
}

impl Conn {
    fn new(stream: TcpStream) -> Self {
        let (read, writer) = stream.into_split();
        Self {
            lines: BufReader::new(read).lines(),
            writer,
        }
    }

    async fn recv(&mut self) -> String {
        timeout(STEP, self.lines.next_line())
            .await
            .expect("timed out waiting for the client")
            .unwrap()
            .expect("client closed the connection")
    }

    async fn expect(&mut self, expected: &str) {
        let line = self.recv().await;
        assert_eq!(line, expected);
    }

    async fn send(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .unwrap();
    }

    /// Registration without CAP support beyond what is passed.
    async fn register(&mut self, nick: &str, caps: &str) {
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
    async fn closed(&mut self) {
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

fn config(port: u16, extra: &str, log_dir: &Path) -> Config {
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

fn fast(ping_interval: Duration) -> SettingsHook {
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
async fn until(client: &mut Client, mut done: impl FnMut(&Client) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(client) {
        assert!(Instant::now() < deadline, "condition never held");
        let _ = timeout(Duration::from_millis(20), client.next()).await;
    }
}

/// Drives the client until the mock server's script has finished. The client
/// must keep running, as effects such as rejoining come from the app loop.
async fn drive<T>(client: &mut Client, server: tokio::task::JoinHandle<T>) -> T {
    until(client, |_| server.is_finished()).await;
    server.await.unwrap()
}

fn server_text(client: &Client) -> Vec<String> {
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

fn status(client: &Client) -> Status {
    client.state().networks().next().unwrap().status
}

#[tokio::test]
async fn registers_with_cap_and_sasl_plain() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(
        mock.port(),
        r#"
        [network.sasl]
        mechanism = "plain"
        account = "acct"
        password = { plain = "sesame" }
        "#,
        dir.path(),
    );
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.expect("CAP LS 302").await;
        c.expect("NICK me").await;
        c.expect("USER me 0 * me").await;
        c.send(":srv CAP * LS * :sasl=PLAIN server-time").await;
        c.send(":srv CAP * LS :away-notify").await;
        c.expect("CAP REQ :sasl server-time").await;
        c.send(":srv CAP * ACK :sasl server-time").await;
        c.expect("AUTHENTICATE PLAIN").await;
        c.send("AUTHENTICATE +").await;
        // base64("acct\0acct\0sesame")
        c.expect("AUTHENTICATE YWNjdABhY2N0AHNlc2FtZQ==").await;
        c.send(":srv 900 me me!u@h acct :You are now logged in as acct")
            .await;
        c.send(":srv 903 me :SASL authentication successful").await;
        c.expect("CAP END").await;
        c.send(":srv 001 me :Welcome").await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send("@time=2026-01-01T00:00:00Z :bob!b@h PRIVMSG #test :hello there")
            .await;
        c
    });

    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| {
        c.state()
            .networks()
            .next()
            .and_then(|n| n.channel("#test"))
            .is_some_and(|ch| {
                c.state()
                    .buffer(ch.buffer)
                    .is_some_and(|b| b.lines().len() >= 2)
            })
    })
    .await;
    let net = client.state().networks().next().unwrap();
    assert_eq!(net.status, Status::Registered);
    assert_eq!(net.account.as_deref(), Some("acct"));
    assert_eq!(net.caps, ["sasl", "server-time"]);
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l == "SASL authentication succeeded as acct")
    );
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l.contains("Plaintext secret"))
    );

    let mut conn = server.await.unwrap();
    client.quit(Some("bye")).await;
    conn.expect("QUIT bye").await;

    let log = std::fs::read_to_string(dir.path().join("mock").join("#test.log")).unwrap();
    assert!(
        log.contains("2026-01-01T00:00:00Z <bob> hello there"),
        "{log}"
    );
}

/// A server that lists its mechanisms without ours is never asked to
/// authenticate, so SCRAM can't be downgraded to PLAIN.
#[tokio::test]
async fn sasl_mechanism_not_offered_is_not_attempted() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(
        mock.port(),
        "[network.sasl]\nmechanism = \"scram-sha-256\"\naccount = \"a\"\npassword = { plain = \"pw\" }",
        dir.path(),
    );
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        for _ in 0..3 {
            c.recv().await;
        }
        c.send(":srv CAP * LS :sasl=PLAIN,EXTERNAL").await;
        c.expect("CAP REQ sasl").await;
        c.send(":srv CAP * ACK :sasl").await;
        // No AUTHENTICATE: straight to CAP END.
        c.expect("CAP END").await;
        c.send(":srv 001 me :Welcome").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| status(c) == Status::Registered).await;
    assert!(
        server_text(&client).iter().any(|l| l
            == "SASL authentication failed: the server offers SASL PLAIN,EXTERNAL but not SCRAM-SHA-256"),
        "{:?}",
        server_text(&client)
    );
    let _conn = server.await.unwrap();
    client.quit(None).await;
}

#[tokio::test]
async fn sasl_failure_still_registers() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(
        mock.port(),
        "[network.sasl]\nmechanism = \"plain\"\naccount = \"a\"\npassword = { plain = \"wrong\" }",
        dir.path(),
    );
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        for _ in 0..3 {
            c.recv().await;
        }
        c.send(":srv CAP * LS :sasl").await;
        c.expect("CAP REQ sasl").await;
        c.send(":srv CAP * ACK :sasl").await;
        c.expect("AUTHENTICATE PLAIN").await;
        c.send("AUTHENTICATE +").await;
        c.recv().await;
        c.send(":srv 904 me :SASL authentication failed").await;
        c.expect("CAP END").await;
        c.send(":srv 001 me :Welcome").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| status(c) == Status::Registered).await;
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l == "SASL authentication failed: SASL authentication failed")
    );
    let _conn = server.await.unwrap();
    client.quit(None).await;
}

#[tokio::test]
async fn falls_back_through_nicks() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.expect("CAP LS 302").await;
        c.expect("NICK me").await;
        c.expect("USER me 0 * me").await;
        c.send(":srv 433 * me :Nickname is already in use").await;
        c.expect("NICK me_").await;
        c.send(":srv 433 * me_ :Nickname is already in use").await;
        c.expect("NICK me1").await;
        c.send(":srv 421 * CAP :Unknown command").await;
        c.send(":srv 001 me1 :Welcome").await;
        c.expect("JOIN #test").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| status(c) == Status::Registered).await;
    assert_eq!(client.state().networks().next().unwrap().nick, "me1");
    let _conn = server.await.unwrap();
    client.quit(None).await;
}

#[tokio::test]
async fn answers_pings_and_reconnects_after_ping_timeout() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send("PING :are-you-there").await;
        c.expect("PONG are-you-there").await;
        // Now go silent: the client pings, gets nothing, and gives up.
        let ping = c.recv().await;
        assert!(ping.starts_with("PING irsty-"), "{ping}");
        c.closed().await;
        // It reconnects and rejoins.
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_millis(300))));
    until(&mut client, |c| {
        server_text(c).iter().any(|l| l.contains("ping timeout"))
    })
    .await;
    until(&mut client, |c| status(c) == Status::Registered).await;
    let _conn = server.await.unwrap();
    client.quit(None).await;
}

#[tokio::test]
async fn reconnects_after_abrupt_disconnect_and_rejoins() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send(":me!u@h JOIN #extra").await;
        // Give the client a moment to apply the joins, then vanish.
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(c);
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test,#extra").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| {
        server_text(c)
            .iter()
            .any(|l| l.contains("connection closed by server"))
    })
    .await;
    let extra = client
        .state()
        .networks()
        .next()
        .unwrap()
        .channel("#extra")
        .unwrap();
    assert!(!extra.joined && extra.rejoin);
    let _conn = drive(&mut client, server).await;
    client.quit(None).await;
}

#[tokio::test]
async fn survives_hostile_input() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        let huge = "A".repeat(irsty_proto::MAX_LINE_LEN * 2);
        c.send(&huge).await;
        c.send(":\r\n\0\0garbage\r\n@only-tags").await;
        c.writer
            .write_all(b":bob!b@h PRIVMSG #test :caf\xe9 \x1b]0;pwned\x07\r\n")
            .await
            .unwrap();
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| {
        c.state()
            .networks()
            .next()
            .and_then(|n| n.channel("#test"))
            .and_then(|ch| c.state().buffer(ch.buffer))
            .is_some_and(|b| !b.lines().is_empty())
    })
    .await;
    let net = client.state().networks().next().unwrap();
    let buffer = client
        .state()
        .buffer(net.channel("#test").unwrap().buffer)
        .unwrap();
    let text = buffer.lines().back().unwrap().text.to_plain();
    assert!(text.starts_with("café "), "{text}");
    assert!(!text.chars().any(char::is_control), "{text:?}");
    assert_eq!(net.status, Status::Registered, "connection survived");
    let _conn = server.await.unwrap();
    client.quit(None).await;
}

#[tokio::test]
async fn rate_limits_output_in_order() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "autoconnect = true", dir.path());
    let hook: SettingsHook = Arc::new(|s| {
        s.rate.burst = 2;
        s.rate.interval = Duration::from_millis(150);
        s.backoff_base = Duration::from_millis(50);
    });
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        let start = Instant::now();
        for i in 0..5 {
            c.expect(&format!("PRIVMSG #test {i}")).await;
        }
        start.elapsed()
    });
    let mut client = Client::start(&cfg, None, Some(hook));
    until(&mut client, |c| status(c) == Status::Registered).await;
    let id = client.state().networks().next().unwrap().id;
    for i in 0..5 {
        client
            .send(
                id,
                Message::new("PRIVMSG").param("#test").param(i.to_string()),
            )
            .unwrap();
    }
    let elapsed = server.await.unwrap();
    // Registration used the burst, so every message waits its turn.
    assert!(elapsed >= Duration::from_millis(500), "{elapsed:?}");
    client.quit(None).await;
}

#[tokio::test]
async fn reports_missing_sasl_and_bad_secrets() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(
        mock.port(),
        "[network.sasl]\nmechanism = \"plain\"\naccount = \"a\"\npassword = { plain = \"p\" }",
        dir.path(),
    );
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "server-time").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| status(c) == Status::Registered).await;
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l.contains("does not offer SASL"))
    );
    let _conn = server.await.unwrap();
    client.quit(None).await;

    let bad = config(
        1,
        "[network.sasl]\nmechanism = \"plain\"\naccount = \"a\"\npassword = { command = [\"false\"] }",
        dir.path(),
    );
    let mut client = Client::start(&bad, None, None);
    until(&mut client, |c| {
        !c.is_running(c.state().networks().next().unwrap().id)
    })
    .await;
    let text = server_text(&client);
    assert!(
        text.iter()
            .any(|l| l.starts_with("Not connecting: SASL password")),
        "{text:?}"
    );
    let kinds: Vec<_> = client
        .state()
        .buffers()
        .flat_map(|b| b.lines().iter().map(|l| l.kind))
        .collect();
    assert!(kinds.contains(&LineKind::Error));
}

#[tokio::test]
async fn connection_refused_backs_off_and_quit_stops_it() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(port, "", dir.path());
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| {
        server_text(c)
            .iter()
            .filter(|l| l.contains("cannot connect"))
            .count()
            >= 2
    })
    .await;
    assert_eq!(status(&client), Status::Waiting);
    client.quit(None).await;
    assert!(client.next().await.is_none(), "no connections remain");
}

#[tokio::test]
async fn says_split_text_and_reconnects_after_disconnect() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        let mut received = Vec::new();
        loop {
            let line = c.recv().await;
            if line.starts_with("QUIT") {
                assert_eq!(line, "QUIT :see you");
                break;
            }
            received.push(line);
        }
        drop(c);
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        received
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let id = client.state().networks().next().unwrap().id;
    assert_eq!(
        client.say(id, "#test", "too early", false),
        Err(irsty_core::client::SendError::NotConnected)
    );
    until(&mut client, |c| status(c) == Status::Registered).await;

    let long = "word ".repeat(200);
    let buffer = client
        .say(id, "#test", long.trim_end(), false)
        .unwrap()
        .unwrap();
    client.say(id, "#test", "waves", true).unwrap();
    let lines = client.state().buffer(buffer).unwrap().lines().len();
    assert!(lines >= 4, "split into several recorded lines, got {lines}");

    client.disconnect(id, Some("see you")).unwrap();
    until(&mut client, |c| !c.is_running(id)).await;
    assert_eq!(status(&client), Status::Disconnected);
    assert_eq!(client.connect(id), Ok(()));
    assert_eq!(
        client.connect(id),
        Err(irsty_core::client::SendError::AlreadyConnected)
    );
    let received = drive(&mut client, server).await;

    let privmsgs: Vec<&String> = received
        .iter()
        .filter(|l| l.starts_with("PRIVMSG #test :word"))
        .collect();
    assert!(privmsgs.len() >= 3, "{received:?}");
    assert!(privmsgs.iter().all(|l| l.len() <= 510));
    let rejoined: String = privmsgs
        .iter()
        .map(|l| l.trim_start_matches("PRIVMSG #test :"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(rejoined, long.trim_end());
    assert!(received.contains(&"PRIVMSG #test :\x01ACTION waves\x01".to_owned()));
    client.quit(None).await;
}

/// Connects through a SOCKS5 proxy with a username and password. The server
/// name can't resolve locally, so registering proves the proxy did the lookup.
#[tokio::test]
async fn connects_through_a_socks5_proxy() {
    use tokio::io::AsyncReadExt;

    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = proxy.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let text = format!(
        r#"
        [logging]
        dir = "{}"

        [[network]]
        name = "mock"
        nicks = ["me"]

        [network.proxy]
        host = "127.0.0.1"
        port = {port}
        username = "user"
        password = {{ plain = "pass" }}

        [[network.servers]]
        host = "irc.invalid"
        port = 6667
        tls = false
        "#,
        dir.path().display()
    );
    let cfg = Config::parse(&text, Path::new("test.toml")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = timeout(STEP, proxy.accept()).await.unwrap().unwrap();
        let mut greeting = [0u8; 4];
        stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 2, 0, 2]);
        stream.write_all(&[5, 2]).await.unwrap();
        let mut auth = [0u8; 11];
        stream.read_exact(&mut auth).await.unwrap();
        assert_eq!(&auth, b"\x01\x04user\x04pass");
        stream.write_all(&[1, 0]).await.unwrap();
        let mut request = vec![0u8; 5 + "irc.invalid".len() + 2];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[..5], [5, 1, 0, 3, 11]);
        assert_eq!(&request[5..16], b"irc.invalid");
        assert_eq!(&request[16..], 6667u16.to_be_bytes());
        stream
            .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
            .await
            .unwrap();
        let mut conn = Conn::new(stream);
        conn.register("me", "").await;
        conn
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    until(&mut client, |c| status(c) == Status::Registered).await;
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l
                == &format!("Connecting to irc.invalid:6667 via SOCKS5 proxy 127.0.0.1:{port}")),
        "{:?}",
        server_text(&client)
    );
    let _conn = server.await.unwrap();
    client.quit(None).await;
}
