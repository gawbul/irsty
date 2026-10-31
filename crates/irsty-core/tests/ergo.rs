//! Integration tests against a real Ergo server.
//!
//! Skipped unless `IRSTY_ERGO_PORT` (plain-text port) is set. `IRSTY_ERGO_HOST`
//! defaults to 127.0.0.1 and `IRSTY_ERGO_TLS_PORT` enables the TLS tests. CI
//! runs Ergo as a service container; locally, see CONTRIBUTING.md.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stderr,
    reason = "test helpers outside #[test] functions"
)]

use std::path::Path;
use std::time::Duration;

use irsty_core::state::{LineKind, Status};
use irsty_core::{Client, Config};
use irsty_proto::Message;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};

struct Ergo {
    host: String,
    port: u16,
    tls_port: Option<u16>,
}

fn ergo() -> Option<Ergo> {
    let Ok(port) = std::env::var("IRSTY_ERGO_PORT") else {
        eprintln!("IRSTY_ERGO_PORT not set; skipping Ergo integration test");
        return None;
    };
    Some(Ergo {
        host: std::env::var("IRSTY_ERGO_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: port.parse().expect("IRSTY_ERGO_PORT must be a port number"),
        tls_port: std::env::var("IRSTY_ERGO_TLS_PORT")
            .ok()
            .map(|p| p.parse().expect("bad IRSTY_ERGO_TLS_PORT")),
    })
}

/// A short name unique to this test run.
fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!(
        "{prefix}{:x}{:x}",
        std::process::id() % 0xfff,
        nanos % 0xffff
    )
}

/// Registers an account with `NickServ` over a raw connection.
async fn register_account(ergo: &Ergo, nick: &str, password: &str) {
    let stream = TcpStream::connect((ergo.host.as_str(), ergo.port))
        .await
        .unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write
        .write_all(format!("NICK {nick}\r\nUSER {nick} 0 * {nick}\r\n").as_bytes())
        .await
        .unwrap();
    let wait_for = async |lines: &mut tokio::io::Lines<BufReader<_>>, needle: &str| {
        timeout(Duration::from_secs(10), async {
            while let Some(line) = lines.next_line().await.unwrap() {
                if line.contains(needle) {
                    return;
                }
            }
            panic!("connection closed before {needle:?}");
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"));
    };
    wait_for(&mut lines, " 001 ").await;
    write
        .write_all(format!("PRIVMSG NickServ :REGISTER {password}\r\n").as_bytes())
        .await
        .unwrap();
    // Ergo confirms with a NickServ notice rather than a numeric.
    wait_for(&mut lines, ":Account created").await;
    write.write_all(b"QUIT\r\n").await.unwrap();
}

fn config(ergo: &Ergo, networks: &str, log_dir: &Path) -> Config {
    let text = format!(
        "[logging]\ndir = \"{}\"\n{}",
        log_dir.display(),
        networks
            .replace("{host}", &ergo.host)
            .replace("{port}", &ergo.port.to_string())
            .replace(
                "{tls_port}",
                &ergo.tls_port.map(|p| p.to_string()).unwrap_or_default()
            )
    );
    Config::parse(&text, Path::new("ergo-test.toml")).unwrap()
}

async fn until(client: &mut Client, what: &str, mut done: impl FnMut(&Client) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(client) {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        let _ = timeout(Duration::from_millis(50), client.next()).await;
    }
}

fn texts(client: &Client, network: &str, buffer: Option<&str>) -> Vec<String> {
    let state = client.state();
    let net = state.networks().find(|n| n.name == network).unwrap();
    let id = match buffer {
        Some(name) => match net.channel(name) {
            Some(channel) => channel.buffer,
            None => return Vec::new(),
        },
        None => net.server_buffer,
    };
    state
        .buffer(id)
        .unwrap()
        .lines()
        .iter()
        .map(|l| l.text.to_plain())
        .collect()
}

fn network<'a>(client: &'a Client, name: &str) -> &'a irsty_core::state::Network {
    client.state().networks().find(|n| n.name == name).unwrap()
}

#[tokio::test]
async fn sasl_plain_against_ergo() {
    let Some(ergo) = ergo() else { return };
    let nick = unique("sasl");
    let password = unique("pw");
    register_account(&ergo, &nick, &password).await;

    let dir = tempfile::tempdir().unwrap();
    let cfg = config(
        &ergo,
        &format!(
            r#"
            [[network]]
            name = "ergo"
            nicks = ["{nick}"]
            [[network.servers]]
            host = "{{host}}"
            port = {{port}}
            tls = false
            [network.sasl]
            mechanism = "plain"
            account = "{nick}"
            password = {{ plain = "{password}" }}
            "#
        ),
        dir.path(),
    );
    let mut client = Client::start(&cfg, None, None);
    until(&mut client, "registration", |c| {
        network(c, "ergo").status == Status::Registered
    })
    .await;
    let net = network(&client, "ergo");
    assert_eq!(net.account.as_deref(), Some(nick.as_str()));
    assert!(net.caps.iter().any(|c| c == "sasl"));
    assert!(net.caps.iter().any(|c| c == "server-time"));
    client.quit(Some("done")).await;

    // A wrong password is reported and registration still completes.
    let wrong = config(
        &ergo,
        &format!(
            r#"
            [[network]]
            name = "ergo"
            nicks = ["{nick}x"]
            [[network.servers]]
            host = "{{host}}"
            port = {{port}}
            tls = false
            [network.sasl]
            mechanism = "plain"
            account = "{nick}"
            password = {{ plain = "not-it" }}
            "#
        ),
        dir.path(),
    );
    let mut client = Client::start(&wrong, None, None);
    until(&mut client, "registration", |c| {
        network(c, "ergo").status == Status::Registered
    })
    .await;
    assert!(network(&client, "ergo").account.is_none());
    assert!(
        texts(&client, "ergo", None)
            .iter()
            .any(|l| l.starts_with("SASL authentication failed"))
    );
    client.quit(None).await;
}

#[tokio::test]
async fn two_clients_join_talk_and_part() {
    let Some(ergo) = ergo() else { return };
    let channel = format!("#{}", unique("chan"));
    let (alice, bob) = (unique("a"), unique("b"));
    let dir = tempfile::tempdir().unwrap();
    let net = |name: &str, nick: &str| {
        format!(
            r#"
            [[network]]
            name = "{name}"
            nicks = ["{nick}"]
            autojoin = ["{channel}"]
            [[network.servers]]
            host = "{{host}}"
            port = {{port}}
            tls = false
            "#
        )
    };
    let cfg = config(
        &ergo,
        &(net("alice", &alice) + &net("bob", &bob)),
        dir.path(),
    );
    let mut client = Client::start(&cfg, None, None);

    until(&mut client, "both joined and see each other", |c| {
        ["alice", "bob"].iter().all(|n| {
            network(c, n)
                .channel(&channel)
                .is_some_and(|ch| ch.joined && ch.members().count() == 2)
        })
    })
    .await;

    let alice_id = network(&client, "alice").id;
    client
        .send(
            alice_id,
            Message::new("PRIVMSG")
                .param(&channel)
                .param("hello from alice"),
        )
        .unwrap();
    until(&mut client, "bob sees the message", |c| {
        texts(c, "bob", Some(&channel))
            .iter()
            .any(|l| l == "hello from alice")
    })
    .await;
    let bob_line = {
        let state = client.state();
        let ch = network(&client, "bob").channel(&channel).unwrap();
        state
            .buffer(ch.buffer)
            .unwrap()
            .lines()
            .back()
            .unwrap()
            .clone()
    };
    assert_eq!(bob_line.kind, LineKind::Message);
    assert_eq!(bob_line.sender.as_deref(), Some(alice.as_str()));

    let bob_id = network(&client, "bob").id;
    client
        .send(bob_id, Message::new("PART").param(&channel).param("bye"))
        .unwrap();
    until(&mut client, "alice sees bob leave", |c| {
        network(c, "alice")
            .channel(&channel)
            .is_some_and(|ch| ch.members().count() == 1)
    })
    .await;
    assert!(
        texts(&client, "alice", Some(&channel))
            .iter()
            .any(|l| l.starts_with(&format!("{bob} has left {channel}")))
    );

    // Reconnecting rejoins the channel.
    client.reconnect(alice_id).unwrap();
    until(&mut client, "alice disconnected", |c| {
        texts(c, "alice", None)
            .iter()
            .any(|l| l.contains("Disconnected (reconnecting)"))
    })
    .await;
    until(&mut client, "alice back in the channel", |c| {
        network(c, "alice").status == Status::Registered
            && network(c, "alice")
                .channel(&channel)
                .is_some_and(|ch| ch.joined)
    })
    .await;

    client.quit(Some("done")).await;
    let log = std::fs::read_to_string(
        dir.path()
            .join("bob")
            .join(format!("{}.log", channel.to_lowercase())),
    )
    .unwrap();
    assert!(
        log.contains(&format!("<{alice}> hello from alice")),
        "{log}"
    );
}

#[tokio::test]
async fn tls_verification_is_on_by_default() {
    let Some(ergo) = ergo() else { return };
    let Some(_) = ergo.tls_port else {
        eprintln!("IRSTY_ERGO_TLS_PORT not set; skipping TLS test");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let nick = unique("tls");
    // Ergo's test certificate is self-signed, so a verified connection must fail.
    let verified = config(
        &ergo,
        &format!(
            "[[network]]\nname = \"ergo\"\nnicks = [\"{nick}\"]\n[[network.servers]]\nhost = \"{{host}}\"\nport = {{tls_port}}\n"
        ),
        dir.path(),
    );
    let mut client = Client::start(&verified, None, None);
    until(&mut client, "certificate rejection", |c| {
        texts(c, "ergo", None)
            .iter()
            .any(|l| l.contains("cannot connect") && l.to_lowercase().contains("certificate"))
    })
    .await;
    assert_ne!(network(&client, "ergo").status, Status::Registered);
    client.quit(None).await;

    // With verification explicitly off, it connects and warns loudly.
    let insecure = config(
        &ergo,
        &format!(
            "[[network]]\nname = \"ergo\"\nnicks = [\"{nick}\"]\n[[network.servers]]\nhost = \"{{host}}\"\nport = {{tls_port}}\ntls_verify = false\n"
        ),
        dir.path(),
    );
    let mut client = Client::start(&insecure, None, None);
    until(&mut client, "registration over TLS", |c| {
        network(c, "ergo").status == Status::Registered
    })
    .await;
    assert!(network(&client, "ergo").insecure);
    assert!(
        texts(&client, "ergo", None)
            .iter()
            .any(|l| l.starts_with("WARNING: TLS certificate verification is disabled"))
    );
    client.quit(None).await;
}

#[tokio::test]
async fn sasl_scram_against_ergo() {
    let Some(ergo) = ergo() else { return };
    let nick = unique("scram");
    let password = unique("pw");
    register_account(&ergo, &nick, &password).await;

    let dir = tempfile::tempdir().unwrap();
    let net = |nick: &str, password: &str| {
        format!(
            r#"
            [[network]]
            name = "ergo"
            nicks = ["{nick}"]
            [[network.servers]]
            host = "{{host}}"
            port = {{port}}
            tls = false
            [network.sasl]
            mechanism = "scram-sha-256"
            account = "{account}"
            password = {{ plain = "{password}" }}
            "#,
            account = nick.trim_end_matches('x'),
        )
    };
    let mut client = Client::start(
        &config(&ergo, &net(&nick, &password), dir.path()),
        None,
        None,
    );
    until(&mut client, "registration", |c| {
        network(c, "ergo").status == Status::Registered
    })
    .await;
    assert_eq!(
        network(&client, "ergo").account.as_deref(),
        Some(nick.as_str())
    );
    client.quit(None).await;

    // A wrong password fails, and registration still completes.
    let wrong = config(&ergo, &net(&format!("{nick}x"), "not-it"), dir.path());
    let mut client = Client::start(&wrong, None, None);
    until(&mut client, "registration", |c| {
        network(c, "ergo").status == Status::Registered
    })
    .await;
    assert!(network(&client, "ergo").account.is_none());
    assert!(
        texts(&client, "ergo", None)
            .iter()
            .any(|l| l.starts_with("SASL authentication failed")),
        "{:?}",
        texts(&client, "ergo", None)
    );
    client.quit(None).await;
}

/// CertFP: attach a client certificate to an account, then log in with it
/// through SASL EXTERNAL. Ergo's test certificate is self-signed, hence
/// `tls_verify = false`.
#[tokio::test]
async fn certfp_login_against_ergo() {
    let Some(ergo) = ergo() else { return };
    if ergo.tls_port.is_none() {
        eprintln!("IRSTY_ERGO_TLS_PORT not set; skipping CertFP test");
        return;
    }
    let nick = unique("cfp");
    let password = unique("pw");
    register_account(&ergo, &nick, &password).await;

    let dir = tempfile::tempdir().unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec![nick.clone()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let (cert_path, key_path) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key.serialize_pem()).unwrap();
    let net = |nick: &str, sasl: &str| {
        format!(
            r#"
            [[network]]
            name = "ergo"
            nicks = ["{nick}"]
            [[network.servers]]
            host = "{{host}}"
            port = {{tls_port}}
            tls_verify = false
            [network.client_cert]
            cert = "{cert}"
            key = "{key}"
            [network.sasl]
            {sasl}
            "#,
            cert = cert_path.display(),
            key = key_path.display(),
        )
    };

    // Log in with the password, presenting the certificate, and add it.
    let plain = format!(
        "mechanism = \"plain\"\naccount = \"{nick}\"\npassword = {{ plain = \"{password}\" }}"
    );
    let mut client = Client::start(&config(&ergo, &net(&nick, &plain), dir.path()), None, None);
    until(&mut client, "registration", |c| {
        network(c, "ergo").status == Status::Registered
    })
    .await;
    let id = network(&client, "ergo").id;
    client
        .send(
            id,
            Message::new("PRIVMSG").param("NickServ").param("CERT ADD"),
        )
        .unwrap();
    // NickServ answers in a query buffer.
    until(&mut client, "NickServ confirms CERT ADD", |c| {
        c.state().buffers().any(|b| {
            b.name == "NickServ"
                && b.lines()
                    .iter()
                    .any(|l| l.text.to_plain().contains("successfully added"))
        })
    })
    .await;
    client.quit(None).await;

    // Now the certificate alone logs in.
    let external = "mechanism = \"external\"";
    let mut client = Client::start(
        &config(&ergo, &net(&format!("{nick}e"), external), dir.path()),
        None,
        None,
    );
    until(&mut client, "registration", |c| {
        network(c, "ergo").status == Status::Registered
    })
    .await;
    assert_eq!(
        network(&client, "ergo").account.as_deref(),
        Some(nick.as_str()),
        "{:?}",
        texts(&client, "ergo", None)
    );
    client.quit(None).await;
}

/// The M5 extensions against a real server: the capabilities are enabled,
/// our message is confirmed by its labelled echo, and away-notify reaches the
/// other client.
#[tokio::test]
async fn ircv3_extensions_against_ergo() {
    let Some(ergo) = ergo() else { return };
    let channel = format!("#{}", unique("v3"));
    let (alice, bob) = (unique("a"), unique("b"));
    let dir = tempfile::tempdir().unwrap();
    let net = |name: &str, nick: &str| {
        format!(
            r#"
            [[network]]
            name = "{name}"
            nicks = ["{nick}"]
            autojoin = ["{channel}"]
            [[network.servers]]
            host = "{{host}}"
            port = {{port}}
            tls = false
            "#
        )
    };
    let cfg = config(
        &ergo,
        &(net("alice", &alice) + &net("bob", &bob)),
        dir.path(),
    );
    let mut client = Client::start(&cfg, None, None);
    until(&mut client, "both joined", |c| {
        ["alice", "bob"].iter().all(|n| {
            network(c, n)
                .channel(&channel)
                .is_some_and(|ch| ch.joined && ch.members().count() == 2)
        })
    })
    .await;
    let caps = network(&client, "alice").caps.clone();
    for cap in [
        "account-notify",
        "account-tag",
        "away-notify",
        "batch",
        "chghost",
        "echo-message",
        "extended-join",
        "invite-notify",
        "labeled-response",
        "setname",
        "standard-replies",
    ] {
        assert!(caps.iter().any(|c| c == cap), "{cap} not enabled: {caps:?}");
    }

    let alice_id = network(&client, "alice").id;
    client
        .say(alice_id, &channel, "echoed back", false)
        .unwrap();
    let delivered = |c: &Client| {
        let ch = network(c, "alice").channel(&channel).unwrap();
        c.state()
            .buffer(ch.buffer)
            .unwrap()
            .lines()
            .iter()
            .filter(|l| l.raw == "echoed back")
            .map(|l| l.delivery.clone())
            .collect::<Vec<_>>()
    };
    until(&mut client, "echo confirms the line", |c| {
        delivered(c) == [irsty_core::state::Delivery::Delivered]
    })
    .await;

    let bob_id = network(&client, "bob").id;
    client
        .send(bob_id, Message::new("AWAY").param("lunch"))
        .unwrap();
    until(&mut client, "alice sees bob away", |c| {
        network(c, "alice")
            .user(&bob)
            .is_some_and(|u| u.away.as_deref() == Some("lunch"))
    })
    .await;
    client.quit(None).await;
}

/// M9 against a real server: a client joining later gets the channel's
/// history, without notifications, and read markers round-trip.
#[tokio::test]
async fn chathistory_and_read_markers_against_ergo() {
    let Some(ergo) = ergo() else { return };
    let channel = format!("#{}", unique("hist"));
    let (alice, bob) = (unique("a"), unique("b"));
    let dir = tempfile::tempdir().unwrap();
    let net = |name: &str, nick: &str| {
        format!(
            r#"
            [[network]]
            name = "{name}"
            nicks = ["{nick}"]
            autojoin = ["{channel}"]
            [[network.servers]]
            host = "{{host}}"
            port = {{port}}
            tls = false
            "#
        )
    };
    // Alice talks first, alone.
    let mut first = Client::start(
        &config(&ergo, &net("alice", &alice), dir.path()),
        None,
        None,
    );
    until(&mut first, "alice joined", |c| {
        network(c, "alice")
            .channel(&channel)
            .is_some_and(|ch| ch.joined)
    })
    .await;
    let alice_id = network(&first, "alice").id;
    for text in ["first message", &format!("{bob}: are you there?")] {
        first.say(alice_id, &channel, text, false).unwrap();
    }
    until(&mut first, "echoes", |c| {
        texts(c, "alice", Some(&channel))
            .iter()
            .any(|l| l.ends_with("are you there?"))
    })
    .await;
    let caps = network(&first, "alice").caps.clone();
    assert!(caps.iter().any(|c| c == "draft/chathistory"), "{caps:?}");

    // Bob joins later and gets the history, without being notified.
    let mut second = Client::start(&config(&ergo, &net("bob", &bob), dir.path()), None, None);
    until(&mut second, "bob sees the history", |c| {
        texts(c, "bob", Some(&channel))
            .iter()
            .any(|l| l == "first message")
    })
    .await;
    assert!(
        second.take_notifications().is_empty(),
        "history doesn't notify"
    );

    // Reading marks it on the server; Ergo echoes the marker back.
    let buffer = network(&second, "bob").channel(&channel).unwrap().buffer;
    second.mark_read(buffer);
    until(&mut second, "read marker", |c| {
        c.state().buffer(buffer).unwrap().read_marker.is_some()
    })
    .await;
    first.quit(None).await;
    second.quit(None).await;
}

/// Paging back: with two messages at a time, older history arrives on top in
/// order, until the server has no more.
#[tokio::test]
async fn older_history_against_ergo() {
    let Some(ergo) = ergo() else { return };
    let channel = format!("#{}", unique("old"));
    let (alice, bob) = (unique("a"), unique("b"));
    let dir = tempfile::tempdir().unwrap();
    let net = |name: &str, nick: &str| {
        format!(
            "[history]\nlines = 2\n[[network]]\nname = \"{name}\"\nnicks = [\"{nick}\"]\nautojoin = [\"{channel}\"]\n[[network.servers]]\nhost = \"{{host}}\"\nport = {{port}}\ntls = false\n"
        )
    };
    let mut first = Client::start(
        &config(&ergo, &net("alice", &alice), dir.path()),
        None,
        None,
    );
    until(&mut first, "alice joined", |c| {
        network(c, "alice")
            .channel(&channel)
            .is_some_and(|ch| ch.joined)
    })
    .await;
    let alice_id = network(&first, "alice").id;
    for n in 1..=6 {
        first
            .say(alice_id, &channel, &format!("message {n}"), false)
            .unwrap();
    }
    // Our own lines show at once as pending; wait until the server has the
    // last one (the rate limiter sends them a couple of seconds apart).
    until(&mut first, "all delivered", |c| {
        let ch = network(c, "alice").channel(&channel).unwrap();
        c.state()
            .buffer(ch.buffer)
            .unwrap()
            .lines()
            .iter()
            .any(|l| l.raw == "message 6" && l.delivery == irsty_core::state::Delivery::Delivered)
    })
    .await;

    let mut second = Client::start(&config(&ergo, &net("bob", &bob), dir.path()), None, None);
    let said = |c: &Client| -> Vec<String> {
        texts(c, "bob", Some(&channel))
            .into_iter()
            .filter(|l| l.starts_with("message "))
            .collect()
    };
    // Ergo counts its HistServ join lines towards the limit, so page back
    // until all six messages are here, then check their order.
    until(&mut second, "some history", |c| !said(c).is_empty()).await;
    let buffer = network(&second, "bob").channel(&channel).unwrap().buffer;
    let all: Vec<String> = (1..=6).map(|n| format!("message {n}")).collect();
    for _ in 0..20 {
        if said(&second) == all {
            break;
        }
        second.request_older(buffer);
        until(&mut second, "a page of history", |c| {
            !c.state().buffer(buffer).unwrap().history_pending
        })
        .await;
    }
    assert_eq!(
        said(&second),
        all,
        "every message, oldest first: {:?}",
        texts(&second, "bob", Some(&channel))
    );
    first.quit(None).await;
    second.quit(None).await;
}

/// M10's acceptance: typing, a reply, a reaction and a multi-line message
/// from one Irsty to another through Ergo.
#[tokio::test]
async fn typing_replies_reactions_and_multiline_against_ergo() {
    let Some(ergo) = ergo() else { return };
    let channel = format!("#{}", unique("tags"));
    let (alice, bob) = (unique("a"), unique("b"));
    let dir = tempfile::tempdir().unwrap();
    let net = |name: &str, nick: &str| {
        format!(
            r#"
            [[network]]
            name = "{name}"
            nicks = ["{nick}"]
            autojoin = ["{channel}"]
            [[network.servers]]
            host = "{{host}}"
            port = {{port}}
            tls = false
            "#
        )
    };
    let cfg = config(
        &ergo,
        &(String::from("[typing]\nsend = true\n") + &net("alice", &alice) + &net("bob", &bob)),
        dir.path(),
    );
    let mut client = Client::start(&cfg, None, None);
    until(&mut client, "both joined and see each other", |c| {
        ["alice", "bob"].iter().all(|n| {
            network(c, n)
                .channel(&channel)
                .is_some_and(|ch| ch.joined && ch.members().count() == 2)
        })
    })
    .await;
    assert!(
        network(&client, "alice").multiline().is_some(),
        "Ergo offers draft/multiline"
    );
    let (alice_id, bob_id) = (network(&client, "alice").id, network(&client, "bob").id);
    let buffer_of = |c: &Client, n: &str| channel_buffer(c, n, &channel);
    let lines_of = |c: &Client, n: &str| channel_lines(c, n, &channel);

    // Bob asks; alice has its msgid.
    client
        .say(bob_id, &channel, "release today?", false)
        .unwrap();
    until(&mut client, "alice sees the question with an id", |c| {
        c.state()
            .last_message_from(buffer_of(c, "alice"), &bob)
            .is_some()
    })
    .await;
    let msgid = client
        .state()
        .last_message_from(buffer_of(&client, "alice"), &bob)
        .and_then(|l| l.msgid.clone())
        .unwrap();

    // Alice types: bob sees it.
    client.typing(alice_id, &channel, true);
    until(&mut client, "bob sees alice typing", |c| {
        c.state()
            .buffer(buffer_of(c, "bob"))
            .unwrap()
            .typing(jiff::Timestamp::now())
            .any(|t| t.nick == alice)
    })
    .await;

    // Alice replies; bob sees what it answers, and alice stops typing.
    client
        .reply(alice_id, &channel, &msgid, "yes, after lunch")
        .unwrap();
    until(&mut client, "bob sees the reply", |c| {
        lines_of(c, "bob").iter().any(|l| {
            l.text.to_plain() == "yes, after lunch"
                && l.reply
                    .as_ref()
                    .and_then(|r| r.parent.as_ref())
                    .is_some_and(|(sender, _)| *sender == bob)
        })
    })
    .await;
    assert_eq!(
        client
            .state()
            .buffer(buffer_of(&client, "bob"))
            .unwrap()
            .typing(jiff::Timestamp::now())
            .count(),
        0,
        "a message ends typing"
    );

    reaction_through_ergo(&mut client, &channel, &msgid, &alice).await;
    multiline_through_ergo(&mut client, alice_id, &channel).await;
    client.quit(Some("done")).await;
}

fn channel_buffer(
    client: &Client,
    network_name: &str,
    channel: &str,
) -> irsty_core::state::BufferId {
    network(client, network_name)
        .channel(channel)
        .unwrap()
        .buffer
}

fn channel_lines(
    client: &Client,
    network_name: &str,
    channel: &str,
) -> Vec<irsty_core::state::Line> {
    client
        .state()
        .buffer(channel_buffer(client, network_name, channel))
        .unwrap()
        .lines()
        .iter()
        .cloned()
        .collect()
}

/// Alice's multi-line message reaches bob as one, in order, and alice's own
/// copy is confirmed by the echo rather than shown twice.
async fn multiline_through_ergo(
    client: &mut Client,
    alice_id: irsty_core::event::NetworkId,
    channel: &str,
) {
    let lines_of = |c: &Client, n: &str| channel_lines(c, n, channel);
    client
        .say(alice_id, channel, "checklist:\n- docs\n- changelog", false)
        .unwrap();
    until(client, "bob sees the multi-line message", |c| {
        lines_of(c, "bob")
            .iter()
            .any(|l| l.text.to_plain() == "- changelog")
    })
    .await;
    let lines = lines_of(client, "bob");
    let first = lines
        .iter()
        .position(|l| l.text.to_plain() == "checklist:")
        .unwrap();
    let texts: Vec<String> = lines[first..first + 3]
        .iter()
        .map(|l| l.text.to_plain())
        .collect();
    assert_eq!(texts, ["checklist:", "- docs", "- changelog"]);
    assert!(!lines[first].continued && lines[first].msgid.is_some());
    assert!(lines[first + 1].continued && lines[first + 2].continued);
    // Alice's own copy was confirmed by the echo, not shown twice.
    until(client, "alice's lines confirmed", |c| {
        let own = lines_of(c, "alice");
        own.iter().filter(|l| l.text.to_plain() == "- docs").count() == 1
            && own
                .iter()
                .all(|l| !matches!(l.delivery, irsty_core::state::Delivery::Pending { .. }))
    })
    .await;
}

/// Alice reacts to bob's message `msgid`; both see it, alice through its
/// echo.
async fn reaction_through_ergo(client: &mut Client, channel: &str, msgid: &str, alice: &str) {
    client
        .react(channel_buffer(client, "alice", channel), msgid, "👍", false)
        .unwrap();
    let reacted = |c: &Client, n: &str| {
        channel_lines(c, n, channel)
            .iter()
            .find(|l| l.msgid.as_deref() == Some(msgid))
            .is_some_and(|l| {
                l.reactions
                    .iter()
                    .any(|r| r.text == "👍" && r.nicks == [alice])
            })
    };
    until(client, "both see the reaction", |c| {
        reacted(c, "bob") && reacted(c, "alice")
    })
    .await;
}
