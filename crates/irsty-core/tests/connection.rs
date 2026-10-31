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
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::{Instant, timeout};

mod support;

use support::*;

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
        c.send(":srv CAP * LS :draft/not-wanted").await;
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

/// Registration isn't held back by the output rate limit. With the default
/// limit (a burst of 5, then one every 2 s), the 7 lines of a SASL login
/// would otherwise take 4 s.
#[tokio::test]
async fn registration_skips_the_rate_limit() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(
        mock.port(),
        "[network.sasl]\nmechanism = \"plain\"\naccount = \"a\"\npassword = { plain = \"pw\" }",
        dir.path(),
    );
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        let start = Instant::now();
        for _ in 0..3 {
            c.recv().await;
        }
        c.send(":srv CAP * LS :sasl").await;
        c.expect("CAP REQ sasl").await;
        c.send(":srv CAP * ACK :sasl").await;
        c.expect("AUTHENTICATE PLAIN").await;
        c.send("AUTHENTICATE +").await;
        c.recv().await;
        c.send(":srv 903 me :SASL authentication successful").await;
        c.expect("CAP END").await;
        let elapsed = start.elapsed();
        c.send(":srv 001 me :Welcome").await;
        (c, elapsed)
    });
    // Default rate limit; only the registration timeout is shortened.
    let hook: SettingsHook = Arc::new(|s| s.timeouts.registration = Duration::from_secs(5));
    let mut client = Client::start(&cfg, None, Some(hook));
    until(&mut client, |c| status(c) == Status::Registered).await;
    let (_conn, elapsed) = server.await.unwrap();
    assert!(
        elapsed < Duration::from_secs(1),
        "registration took {elapsed:?}"
    );
    client.quit(None).await;
}
