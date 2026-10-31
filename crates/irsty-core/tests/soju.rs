//! Integration tests against a real soju bouncer (M9's acceptance).
//!
//! Skipped unless `IRSTY_SOJU_PORT` is set. They expect `mise run soju`
//! (`scripts/soju.sh`): user `irsty` with password `irsty-test` and networks
//! `ergo` and `ergo2` on the local Ergo, whose plain-text port is
//! `IRSTY_ERGO_PORT` (default 16667).

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

use irsty_core::state::Status;
use irsty_core::{Client, Config};
use irsty_proto::Message;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};

fn soju_port() -> Option<u16> {
    let Ok(port) = std::env::var("IRSTY_SOJU_PORT") else {
        eprintln!("IRSTY_SOJU_PORT not set; skipping soju integration test");
        return None;
    };
    Some(port.parse().expect("IRSTY_SOJU_PORT must be a port number"))
}

fn ergo_port() -> u16 {
    std::env::var("IRSTY_ERGO_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(16667)
}

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

fn config(port: u16, log_dir: &Path) -> Config {
    let text = format!(
        r#"
        [logging]
        dir = "{}"

        [[network]]
        name = "soju"
        nicks = ["irsty"]

        [[network.servers]]
        host = "127.0.0.1"
        port = {port}
        tls = false

        [network.sasl]
        mechanism = "plain"
        account = "irsty"
        password = {{ plain = "irsty-test" }}
        "#,
        log_dir.display()
    );
    Config::parse(&text, Path::new("soju-test.toml")).unwrap()
}

async fn until(client: &mut Client, what: &str, mut done: impl FnMut(&Client) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(client) {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        let _ = timeout(Duration::from_millis(50), client.next()).await;
    }
}

fn registered(client: &Client, name: &str) -> bool {
    client
        .state()
        .networks()
        .any(|n| n.name == name && n.status == Status::Registered)
}

fn network_id(client: &Client, name: &str) -> irsty_core::event::NetworkId {
    client
        .state()
        .networks()
        .find(|n| n.name == name)
        .unwrap()
        .id
}

/// Each of the bouncer's networks opens as a network here, bound and
/// registered; one added on the bouncer appears, and deleting it removes it.
#[tokio::test]
async fn bouncer_networks_against_soju() {
    let Some(port) = soju_port() else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::start(&config(port, dir.path()), None, None);
    until(&mut client, "both networks registered", |c| {
        registered(c, "soju/ergo") && registered(c, "soju/ergo2")
    })
    .await;
    // Each is bound to its own upstream: the nicks soju registered with.
    let nick = |c: &Client, name: &str| {
        c.state()
            .networks()
            .find(|n| n.name == name)
            .unwrap()
            .nick
            .clone()
    };
    assert_eq!(nick(&client, "soju/ergo"), "sojutest");
    assert_eq!(nick(&client, "soju/ergo2"), "sojutest2");

    // Add a network on the bouncer, then delete it.
    let control = network_id(&client, "soju");
    let name = unique("tmp");
    let add = Message::new("BOUNCER").param("ADDNETWORK").param(format!(
        "name={name};host=127.0.0.1;port={};tls=0;nickname={name}",
        ergo_port()
    ));
    client.send(control, add).unwrap();
    let full = format!("soju/{name}");
    // The netid comes in the 005 just after registering.
    let netid_of = |c: &Client| {
        c.state()
            .networks()
            .find(|n| n.name == full)
            .and_then(|n| n.isupport.raw.get("BOUNCER_NETID").cloned())
    };
    until(&mut client, "the added network, bound", |c| {
        registered(c, &full) && netid_of(c).is_some()
    })
    .await;
    let netid = netid_of(&client).unwrap();
    client
        .send(
            control,
            Message::new("BOUNCER").param("DELNETWORK").param(netid),
        )
        .unwrap();
    until(&mut client, "the deleted network gone", |c| {
        !c.state().networks().any(|n| n.name == full)
    })
    .await;
    client.quit(None).await;
}

/// History through the bouncer: a message sent while Irsty is away arrives
/// from soju's message store on the next connection, without notifying.
#[tokio::test]
async fn history_through_soju() {
    let Some(port) = soju_port() else { return };
    let dir = tempfile::tempdir().unwrap();
    let channel = format!("#{}", unique("bnc"));

    // Join through the bouncer, so soju keeps the channel.
    let mut client = Client::start(&config(port, dir.path()), None, None);
    until(&mut client, "registered", |c| registered(c, "soju/ergo")).await;
    let id = network_id(&client, "soju/ergo");
    client
        .send(id, Message::new("JOIN").param(&channel))
        .unwrap();
    until(&mut client, "joined", |c| {
        c.state()
            .network(id)
            .and_then(|n| n.channel(&channel))
            .is_some_and(|ch| ch.joined)
    })
    .await;
    client.quit(None).await;

    // Someone talks on Ergo directly while we are gone.
    let speaker = unique("spk");
    let stream = TcpStream::connect(("127.0.0.1", ergo_port()))
        .await
        .unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write
        .write_all(format!("NICK {speaker}\r\nUSER s 0 * s\r\n").as_bytes())
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains(" 001 ") {
                break;
            }
        }
    })
    .await
    .unwrap();
    write
        .write_all(
            format!(
                "JOIN {channel}\r\nPRIVMSG {channel} :sojutest: while you were away\r\nQUIT\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Back again, with nothing in memory: soju rejoins, history fills it in.
    let mut client = Client::start(&config(port, dir.path()), None, None);
    until(&mut client, "the missed message", |c| {
        c.state()
            .networks()
            .filter(|n| n.name == "soju/ergo")
            .any(|n| {
                n.channel(&channel).is_some_and(|ch| {
                    c.state()
                        .buffer(ch.buffer)
                        .unwrap()
                        .lines()
                        .iter()
                        .any(|l| l.text.to_plain() == "sojutest: while you were away")
                })
            })
    })
    .await;
    assert!(
        client.take_notifications().is_empty(),
        "history doesn't notify"
    );
    client.quit(None).await;
}
