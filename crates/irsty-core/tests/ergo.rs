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
    let mut client = Client::start(&cfg, None, None).await;
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
    let mut client = Client::start(&wrong, None, None).await;
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
    let mut client = Client::start(&cfg, None, None).await;

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
    let mut client = Client::start(&verified, None, None).await;
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
    let mut client = Client::start(&insecure, None, None).await;
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
