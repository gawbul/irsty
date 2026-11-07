//! M16: DCC end to end, against a scripted IRC server and raw TCP peers
//! playing the other client: offers, gets with hostile names, truncated
//! transfers and resuming, passive DCC, sends and chats.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions"
)]

mod support;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use irsty_core::Client;
use irsty_core::dcc::Status;
use irsty_proto::dcc::Dcc as Request;
use support::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

async fn connected(dir: &Path) -> (Client, Conn) {
    let mock = Mock::bind().await;
    let extra = format!("\n[dcc]\ndir = \"{}\"\n", dir.join("downloads").display());
    let cfg = config(mock.port(), &extra, &dir.join("logs"));
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let conn = drive(&mut client, server).await;
    (client, conn)
}

fn id(client: &Client) -> irsty_core::event::NetworkId {
    client.state().networks().next().unwrap().id
}

/// The CTCP DCC request bob sends us.
fn offer(request: &Request) -> String {
    format!(":bob!b@h PRIVMSG me :\x01DCC {}\x01", request.encode())
}

/// A peer that serves `data` to whoever connects, reading acks; returns the
/// listener's port, and the last ack seen when it finishes.
async fn serve(data: Vec<u8>, stop_after: Option<usize>) -> (u16, tokio::task::JoinHandle<u32>) {
    let listener = TcpListener::bind((LOCAL, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (mut rd, mut wr) = stream.into_split();
        let send = stop_after.map_or(&data[..], |n| &data[..n]);
        wr.write_all(send).await.unwrap();
        if stop_after.is_some() {
            drop(wr);
        }
        let mut last = 0;
        let mut ack = [0u8; 4];
        while rd.read_exact(&mut ack).await.is_ok() {
            last = u32::from_be_bytes(ack);
            if last as usize == send.len() {
                break;
            }
        }
        last
    });
    (port, task)
}

fn transfer_status(client: &Client) -> Status {
    client.dcc().last().unwrap().status.clone()
}

#[tokio::test]
async fn offers_wait_and_hostile_names_are_made_safe() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, mut conn) = connected(dir.path()).await;
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let (port, peer) = serve(data.clone(), None).await;
    // A file of that name exists already: it must survive.
    let downloads = dir.path().join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("authorized_keys"), b"mine").unwrap();

    conn.send(&offer(&Request::Send {
        file: "../../.ssh/authorized_keys".into(),
        addr: LOCAL,
        port,
        size: Some(data.len() as u64),
        token: None,
    }))
    .await;
    until(&mut client, |c| !c.dcc().is_empty()).await;
    assert_eq!(
        transfer_status(&client),
        Status::Offered,
        "nothing is accepted by itself"
    );
    let note = server_text(&client).last().unwrap().clone();
    assert!(
        note.contains("bob offers the file authorized_keys (200.0 kB)"),
        "{note}"
    );
    assert!(note.contains("its name made safe"));
    assert_eq!(
        client.take_notifications().len(),
        1,
        "an offer is worth telling you about"
    );

    client.dcc_get(id(&client), Some("bob"), None).unwrap();
    until(&mut client, |c| transfer_status(c) == Status::Done).await;
    assert_eq!(
        peer.await.unwrap() as usize,
        data.len(),
        "every byte acknowledged"
    );
    let path = client.dcc().last().unwrap().path.clone().unwrap();
    assert_eq!(
        path,
        downloads.join("authorized_keys.1"),
        "{}",
        path.display()
    );
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(
        std::fs::read(downloads.join("authorized_keys")).unwrap(),
        b"mine"
    );
    assert!(!dir.path().join(".ssh").exists());
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l.starts_with("Received authorized_keys (200.0 kB) from bob"))
    );
    client.quit(None).await;
}

#[tokio::test]
async fn truncated_transfers_resume_from_where_they_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, mut conn) = connected(dir.path()).await;
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 253) as u8).collect();
    let size = data.len() as u64;

    // The first attempt stops after 30,000 bytes.
    let (port, peer) = serve(data.clone(), Some(30_000)).await;
    conn.send(&offer(&Request::Send {
        file: "big file.bin".into(),
        addr: LOCAL,
        port,
        size: Some(size),
        token: None,
    }))
    .await;
    until(&mut client, |c| !c.dcc().is_empty()).await;
    client
        .dcc_get(id(&client), Some("bob"), Some("big file.bin"))
        .unwrap();
    until(&mut client, |c| {
        matches!(transfer_status(c), Status::Failed(_))
    })
    .await;
    peer.await.unwrap();
    let Status::Failed(reason) = transfer_status(&client) else {
        panic!()
    };
    assert!(
        reason.contains("closed after 30.0 kB of 100.0 kB"),
        "{reason}"
    );
    let partial = dir.path().join("downloads/big file.bin.part");
    assert_eq!(std::fs::metadata(&partial).unwrap().len(), 30_000);

    // Offered again: resume asks for the rest.
    let listener = TcpListener::bind((LOCAL, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    conn.send(&offer(&Request::Send {
        file: "big file.bin".into(),
        addr: LOCAL,
        port,
        size: Some(size),
        token: None,
    }))
    .await;
    until(&mut client, |c| transfer_status(c) == Status::Offered).await;
    client.dcc_resume(id(&client), Some("bob"), None).unwrap();
    let resume = conn.recv().await;
    assert_eq!(
        resume,
        format!("PRIVMSG bob :\x01DCC RESUME \"big file.bin\" {port} 30000\x01")
    );
    conn.send(&offer(&Request::Accept {
        file: "big file.bin".into(),
        port,
        position: 30_000,
        token: None,
    }))
    .await;
    let rest = data[30_000..].to_vec();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(&rest).await.unwrap();
        let mut ack = [0u8; 4];
        let mut last = 0;
        while stream.read_exact(&mut ack).await.is_ok() {
            last = u32::from_be_bytes(ack);
            if u64::from(last) == 100_000 {
                break;
            }
        }
        last
    });
    until(&mut client, |c| transfer_status(c) == Status::Done).await;
    assert_eq!(
        peer.await.unwrap(),
        100_000,
        "acks count from the start of the file"
    );
    let done = dir.path().join("downloads/big file.bin");
    assert_eq!(std::fs::read(&done).unwrap(), data);
    assert!(!partial.exists());
    client.quit(None).await;
}

#[tokio::test]
async fn senders_that_send_too_much_are_cut_off_and_bad_offers_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, mut conn) = connected(dir.path()).await;
    let (port, _peer) = serve(vec![7u8; 5000], None).await;
    conn.send(&offer(&Request::Send {
        file: "small".into(),
        addr: LOCAL,
        port,
        size: Some(1000),
        token: None,
    }))
    .await;
    until(&mut client, |c| !c.dcc().is_empty()).await;
    client.dcc_get(id(&client), None, None).unwrap();
    until(&mut client, |c| {
        matches!(transfer_status(c), Status::Failed(_))
    })
    .await;
    let Status::Failed(reason) = transfer_status(&client) else {
        panic!()
    };
    assert!(reason.contains("more than the 1.0 kB offered"), "{reason}");
    let partial = dir.path().join("downloads/small.part");
    assert_eq!(
        std::fs::metadata(partial).unwrap().len(),
        1000,
        "never past the size"
    );

    // An offer pointing at a system port is not connected to.
    conn.send(&offer(&Request::Send {
        file: "x".into(),
        addr: LOCAL,
        port: 22,
        size: Some(1),
        token: None,
    }))
    .await;
    until(&mut client, |c| transfer_status(c) == Status::Offered).await;
    client.dcc_get(id(&client), None, Some("x")).unwrap();
    assert!(matches!(transfer_status(&client), Status::Failed(r) if r.contains("system services")));
    // Declining an offer.
    conn.send(&offer(&Request::Chat {
        addr: LOCAL,
        port: 5000,
        token: None,
    }))
    .await;
    until(&mut client, |c| transfer_status(c) == Status::Offered).await;
    let chat = client.dcc().last().unwrap().id;
    client.dcc_close(&chat.to_string()).unwrap();
    assert!(client.dcc().iter().all(|t| t.id != chat));
    assert!(client.dcc_get(id(&client), Some("nobody"), None).is_err());
    client.quit(None).await;
}

#[tokio::test]
async fn passive_offers_are_answered_by_listening() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, mut conn) = connected(dir.path()).await;
    conn.send(&offer(&Request::Send {
        file: "nat.txt".into(),
        addr: "1.1.1.1".parse().unwrap(),
        port: 0,
        size: Some(11),
        token: Some("42".into()),
    }))
    .await;
    until(&mut client, |c| !c.dcc().is_empty()).await;
    client.dcc_get(id(&client), Some("bob"), None).unwrap();
    // We listen and tell bob where, with his token.
    let reply = conn.recv().await;
    let params = reply
        .strip_prefix("PRIVMSG bob :\x01DCC ")
        .and_then(|r| r.strip_suffix('\x01'))
        .unwrap();
    let Some(Request::Send {
        addr,
        port,
        size,
        token,
        ..
    }) = Request::parse(params)
    else {
        panic!("{reply}")
    };
    assert_eq!(
        (addr, size, token.as_deref()),
        (LOCAL, Some(11), Some("42"))
    );
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l.contains("your IP address (127.0.0.1)"))
    );
    let mut stream = TcpStream::connect(SocketAddr::new(addr, port))
        .await
        .unwrap();
    stream.write_all(b"hello there").await.unwrap();
    until(&mut client, |c| transfer_status(c) == Status::Done).await;
    assert_eq!(
        std::fs::read(dir.path().join("downloads/nat.txt")).unwrap(),
        b"hello there"
    );
    client.quit(None).await;
}

#[tokio::test]
async fn sending_a_file_with_resume() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, mut conn) = connected(dir.path()).await;
    let data: Vec<u8> = (0..50_000u32).map(|i| (i % 249) as u8).collect();
    let file = dir.path().join("report.pdf");
    std::fs::write(&file, &data).unwrap();
    client.dcc_send(id(&client), "bob", &file, false).unwrap();
    let line = conn.recv().await;
    let params = line
        .strip_prefix("PRIVMSG bob :\x01DCC ")
        .and_then(|r| r.strip_suffix('\x01'))
        .unwrap();
    let Some(Request::Send {
        file: name,
        addr,
        port,
        size,
        ..
    }) = Request::parse(params)
    else {
        panic!("{line}")
    };
    assert_eq!((name.as_str(), size), ("report.pdf", Some(50_000)));
    // Bob has the first 20,000 bytes and asks for the rest.
    conn.send(&offer(&Request::Resume {
        file: "report.pdf".into(),
        port,
        position: 20_000,
        token: None,
    }))
    .await;
    until(&mut client, |c| c.dcc().last().unwrap().offset == 20_000).await;
    assert_eq!(
        conn.recv().await,
        format!("PRIVMSG bob :\x01DCC ACCEPT report.pdf {port} 20000\x01")
    );
    let receiver = tokio::spawn(async move {
        let mut stream = TcpStream::connect(SocketAddr::new(addr, port))
            .await
            .unwrap();
        let mut got = Vec::new();
        let mut buf = vec![0u8; 8192];
        let mut total = 20_000u64;
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
            total += n as u64;
            stream
                .write_all(&irsty_proto::dcc::ack(total))
                .await
                .unwrap();
            if total == 50_000 {
                break;
            }
        }
        got
    });
    until(&mut client, |c| transfer_status(c) == Status::Done).await;
    assert_eq!(receiver.await.unwrap(), data[20_000..]);
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l.starts_with("Sent report.pdf (50.0 kB) to bob"))
    );
    client.quit(None).await;
}

#[tokio::test]
async fn chats_both_ways() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, mut conn) = connected(dir.path()).await;
    let net = id(&client);
    client.dcc_chat(net, "bob", false).unwrap();
    let line = conn.recv().await;
    let params = line
        .strip_prefix("PRIVMSG bob :\x01DCC ")
        .and_then(|r| r.strip_suffix('\x01'))
        .unwrap();
    let Some(Request::Chat { addr, port, .. }) = Request::parse(params) else {
        panic!("{line}")
    };
    let stream = TcpStream::connect(SocketAddr::new(addr, port))
        .await
        .unwrap();
    let (rd, mut wr) = stream.into_split();
    until(&mut client, |c| transfer_status(c) == Status::Active).await;
    assert!(
        client.state().find_buffer(net, "=bob").is_some(),
        "the chat's buffer opens when it connects, before anyone speaks"
    );
    // irssi's form of an action, a bare one, and a CTCP that isn't shown.
    wr.write_all(
        b"hi from bob\r\nCTCP_MESSAGE \x01ACTION waves\x01\n\x01ACTION nods\x01\nCTCP_MESSAGE \x01VERSION\x01\n",
    )
    .await
    .unwrap();
    let texts = |c: &Client| -> Vec<String> {
        c.state()
            .find_buffer(net, "=bob")
            .and_then(|b| c.state().buffer(b))
            .map(|b| b.lines().iter().map(|l| l.text.to_plain()).collect())
            .unwrap_or_default()
    };
    until(&mut client, |c| texts(c).len() == 3).await;
    assert_eq!(texts(&client), ["hi from bob", "waves", "nods"]);
    // Typing in =bob goes over the chat, not to the server.
    client.say(net, "=bob", "hello bob", false).unwrap();
    let mut lines = BufReader::new(rd).lines();
    assert_eq!(lines.next_line().await.unwrap().unwrap(), "hello bob");
    assert_eq!(texts(&client).last().unwrap(), "hello bob");
    drop(wr);
    drop(lines);
    until(&mut client, |c| transfer_status(c) == Status::Done).await;
    assert!(
        server_text(&client)
            .iter()
            .any(|l| l == "DCC chat with bob closed")
    );
    assert!(client.say(net, "=bob", "anyone?", false).is_err());
    client.quit(None).await;
}
