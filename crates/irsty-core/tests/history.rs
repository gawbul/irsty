//! M9 end to end against a scripted server: chat history, read markers, soju
//! bouncer networks and ZNC playback.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions"
)]

mod support;

use std::time::Duration;

use irsty_core::Client;
use irsty_core::state::{Activity, BufferId};
use support::*;

const HISTORY_CAPS: &str = "batch server-time message-tags draft/chathistory draft/read-marker";

fn channel(client: &Client) -> Option<BufferId> {
    let net = client.state().networks().next()?;
    net.channel("#test").map(|c| c.buffer)
}

fn texts(client: &Client, buffer: BufferId) -> Vec<String> {
    client
        .state()
        .buffer(buffer)
        .unwrap()
        .lines()
        .iter()
        .map(|l| l.text.to_plain())
        .collect()
}

/// Reads lines until one starts with `prefix`; returns it.
async fn expect_prefix(conn: &mut Conn, prefix: &str) -> String {
    loop {
        let line = conn.recv().await;
        if line.starts_with(prefix) {
            return line;
        }
    }
}

#[tokio::test]
async fn history_on_join_and_older_history() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", HISTORY_CAPS).await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send(":srv MARKREAD #test timestamp=2026-10-31T10:00:30.000Z")
            .await;
        c.send(":srv 366 me #test :End").await;
        // Nothing in the buffer yet, so the latest messages. (TARGETS went
        // out at registration, before this.)
        assert_eq!(
            expect_prefix(&mut c, "CHATHISTORY LATEST").await,
            "CHATHISTORY LATEST #test * 100"
        );
        c.send("BATCH +h chathistory #test").await;
        c.send("@batch=h;time=2026-10-31T10:00:00.000Z;msgid=m1 :alice!a@h PRIVMSG #test :me: missed you").await;
        c.send("@batch=h;time=2026-10-31T10:01:00.000Z;msgid=m2 :bob!b@h PRIVMSG #test :\x01VERSION\x01").await;
        c.send("@batch=h;time=2026-10-31T10:02:00.000Z;msgid=m3 :bob!b@h PRIVMSG #test :later")
            .await;
        c.send("BATCH -h").await;
        // The same message again, as overlapping history would bring it.
        c.send("BATCH +h2 chathistory #test").await;
        c.send("@batch=h2;time=2026-10-31T10:02:00.000Z;msgid=m3 :bob!b@h PRIVMSG #test :later")
            .await;
        c.send("BATCH -h2").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let mut conn = drive(&mut client, server).await;
    until(&mut client, |c| {
        channel(c).is_some_and(|b| texts(c, b).iter().any(|l| l == "later"))
    })
    .await;
    let buffer = channel(&client).unwrap();
    let lines = texts(&client, buffer);
    assert_eq!(
        lines.iter().filter(|l| *l == "later").count(),
        1,
        "deduplicated: {lines:?}"
    );
    assert!(lines.iter().any(|l| l == "me: missed you"));
    assert!(
        client.take_notifications().is_empty(),
        "history never notifies"
    );
    // The CTCP in history was not answered.
    let server_lines = server_text(&client);
    assert!(
        !server_lines.iter().any(|l| l.contains("(answered)")),
        "{server_lines:?}"
    );
    // The read marker from the server.
    let marker = client.state().buffer(buffer).unwrap().read_marker;
    assert_eq!(marker.unwrap().to_string(), "2026-10-31T10:00:30Z");

    // Scrolling past the top asks for older history, which goes on top.
    client.request_older(buffer);
    let line = conn.recv().await;
    assert_eq!(line, "CHATHISTORY BEFORE #test msgid=m1 100");
    client.request_older(buffer);
    conn.send("BATCH +o chathistory #test").await;
    conn.send("@batch=o;time=2026-10-31T09:58:00.000Z;msgid=m0 :carol!c@h PRIVMSG #test :oldest")
        .await;
    conn.send("@batch=o;time=2026-10-31T09:59:00.000Z;msgid=m0b :carol!c@h PRIVMSG #test :older")
        .await;
    conn.send("BATCH -o").await;
    until(&mut client, |c| {
        texts(c, buffer).first().is_some_and(|l| l == "oldest")
    })
    .await;
    assert_eq!(texts(&client, buffer)[1], "older");
    // Only one request went out while the first was pending.
    client.request_older(buffer);
    assert_eq!(conn.recv().await, "CHATHISTORY BEFORE #test msgid=m0 100");
    // An empty answer means there is no more.
    conn.send("BATCH +e chathistory #test").await;
    conn.send("BATCH -e").await;
    until(&mut client, |c| {
        c.state().buffer(buffer).unwrap().history_done
    })
    .await;
    client.request_older(buffer);
    client
        .send(
            client.state().networks().next().unwrap().id,
            irsty_proto::Message::new("PING").param("probe"),
        )
        .unwrap();
    assert_eq!(
        conn.recv().await,
        "PING probe",
        "no request once history is done"
    );
    client.quit(None).await;
}

#[tokio::test]
async fn targets_open_queries_and_read_markers_sync() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", HISTORY_CAPS).await;
        let targets = expect_prefix(&mut c, "CHATHISTORY TARGETS ").await;
        assert!(targets.ends_with(" 50"), "{targets}");
        c.send("BATCH +t draft/chathistory-targets").await;
        c.send("@batch=t CHATHISTORY TARGETS bob timestamp=2026-10-31T10:00:00.000Z")
            .await;
        c.send("@batch=t CHATHISTORY TARGETS #test timestamp=2026-10-31T10:00:00.000Z")
            .await;
        c.send("BATCH -t").await;
        // A query we didn't have is opened and filled.
        c.expect("CHATHISTORY LATEST bob * 100").await;
        c.send("BATCH +q chathistory bob").await;
        c.send("@batch=q;time=2026-10-31T10:00:00.000Z;msgid=q1 :bob!b@h PRIVMSG me :you there?")
            .await;
        c.send("BATCH -q").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let mut conn = drive(&mut client, server).await;
    let id = client.state().networks().next().unwrap().id;
    until(&mut client, |c| {
        c.state()
            .find_buffer(id, "bob")
            .is_some_and(|b| texts(c, b).iter().any(|l| l == "you there?"))
    })
    .await;
    let query = client.state().find_buffer(id, "bob").unwrap();
    assert!(client.take_notifications().is_empty());
    assert_eq!(
        client.state().buffer(query).unwrap().activity,
        Activity::Highlight
    );

    // Another client read it: the unread lines are cleared here too.
    conn.send(":srv MARKREAD bob timestamp=2026-10-31T10:00:00.000Z")
        .await;
    until(&mut client, |c| {
        c.state().buffer(query).unwrap().unread == 0
    })
    .await;
    assert_eq!(
        client.state().buffer(query).unwrap().activity,
        Activity::None
    );

    // Reading a new line here tells the server, once.
    conn.send("@time=2026-10-31T11:00:00.000Z :bob!b@h PRIVMSG me :new")
        .await;
    until(&mut client, |c| texts(c, query).iter().any(|l| l == "new")).await;
    client.mark_read(query);
    assert_eq!(
        conn.recv().await,
        "MARKREAD bob timestamp=2026-10-31T11:00:00.000Z"
    );
    client.mark_read(query);
    client
        .send(id, irsty_proto::Message::new("PING").param("probe"))
        .unwrap();
    assert_eq!(conn.recv().await, "PING probe", "nothing new, nothing sent");
    client.quit(None).await;
}

/// soju: an unbound connection's networks each become a network here, bound
/// with BOUNCER BIND before CAP END, and go away when soju removes them.
#[tokio::test]
async fn bouncer_networks_open_and_close() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut control = mock.accept().await;
        control
            .register(
                "me",
                "batch soju.im/bouncer-networks soju.im/bouncer-networks-notify",
            )
            .await;
        control.send(":srv 005 me CHANTYPES=# :are supported").await;
        control.send("BATCH +n soju.im/bouncer-networks").await;
        control
            .send("@batch=n BOUNCER NETWORK 42 name=Libera\\sChat;state=connected;host=irc.libera.chat")
            .await;
        control.send("BATCH -n").await;
        // The network's own connection binds before ending CAP.
        let mut bound = mock.accept().await;
        bound.expect("CAP LS 302").await;
        bound.expect("NICK me").await;
        bound.expect("USER me 0 * me").await;
        bound.send(":srv CAP * LS :soju.im/bouncer-networks").await;
        bound.expect("CAP REQ soju.im/bouncer-networks").await;
        bound.send(":srv CAP * ACK :soju.im/bouncer-networks").await;
        bound.expect("BOUNCER BIND 42").await;
        bound.expect("CAP END").await;
        bound.send(":srv 001 me :Welcome").await;
        bound
            .send(":srv 005 me BOUNCER_NETID=42 :are supported")
            .await;
        // A bound connection's own BOUNCER NETWORK lines open nothing more.
        bound.send("BOUNCER NETWORK 43 name=Other").await;
        (control, bound)
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let (mut control, _bound) = drive(&mut client, server).await;
    let names =
        |c: &Client| -> Vec<String> { c.state().networks().map(|n| n.name.clone()).collect() };
    until(&mut client, |c| {
        names(c).contains(&"mock/Libera Chat".to_owned())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tokio::time::timeout(Duration::from_millis(100), client.next()).await;
    assert_eq!(names(&client), ["mock", "mock/Libera Chat"]);

    // Removed on the bouncer: removed here.
    control.send("BOUNCER NETWORK 42 *").await;
    until(&mut client, |c| names(c) == ["mock"]).await;
    client.quit(None).await;
}

/// ZNC: with znc.in/playback, ask for what we missed; playback never notifies.
#[tokio::test]
async fn znc_playback() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "batch server-time znc.in/playback").await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        // First connection: nothing seen yet, so everything ZNC has.
        c.expect("PRIVMSG *playback :PLAY * 0").await;
        c.send("BATCH +p znc.in/playback #test").await;
        c.send("@batch=p;time=2026-10-31T09:00:00.000Z :alice!a@h PRIVMSG #test :me: while you were out").await;
        c.send("BATCH -p").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let _conn = drive(&mut client, server).await;
    until(&mut client, |c| {
        channel(c).is_some_and(|b| texts(c, b).iter().any(|l| l == "me: while you were out"))
    })
    .await;
    assert!(client.take_notifications().is_empty());
    client.quit(None).await;
}
