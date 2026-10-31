//! M10 end to end against a scripted server: typing notifications, replies,
//! reactions and multi-line messages, as sent on the wire.

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
use irsty_core::state::{BufferId, Delivery};
use irsty_proto::Message;
use support::*;

const TYPING_ON: &str = "[typing]\nsend = true";

fn channel(client: &Client) -> Option<BufferId> {
    client
        .state()
        .networks()
        .next()?
        .channel("#test")
        .map(|c| c.buffer)
}

/// Sends a PING and expects it next: proof nothing else was sent first.
async fn nothing_sent(client: &Client, conn: &mut Conn, why: &str) {
    let id = client.state().networks().next().unwrap().id;
    client
        .send(id, Message::new("PING").param("probe"))
        .unwrap();
    assert_eq!(conn.recv().await, "PING probe", "{why}");
}

#[tokio::test]
async fn typing_replies_and_reactions_on_the_wire() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), TYPING_ON, dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "message-tags").await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send("@msgid=p1 :alice!a@h PRIVMSG #test :shall we ship it?")
            .await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let mut conn = drive(&mut client, server).await;
    until(&mut client, |c| {
        channel(c).is_some_and(|b| c.state().last_message_from(b, "alice").is_some())
    })
    .await;
    let id = client.state().networks().next().unwrap().id;
    let buffer = channel(&client).unwrap();

    // Typing: at most one notification per target every three seconds, and
    // no "done" so soon after "active".
    client.typing(id, "#test", true);
    assert_eq!(conn.recv().await, "@+typing=active TAGMSG #test");
    client.typing(id, "#test", true);
    client.typing(id, "#test", false);
    client.typing(id, "#test", true);
    nothing_sent(&client, &mut conn, "throttled").await;

    // A reply carries the parent's msgid.
    let msgid = client
        .state()
        .last_message_from(buffer, "alice")
        .and_then(|l| l.msgid.clone())
        .unwrap();
    client.reply(id, "#test", &msgid, "alice: yes").unwrap();
    assert_eq!(conn.recv().await, "@+reply=p1 PRIVMSG #test :alice: yes");
    let own = client
        .state()
        .buffer(buffer)
        .unwrap()
        .lines()
        .back()
        .unwrap()
        .clone();
    let parent = own.reply.unwrap().parent.unwrap();
    assert_eq!(parent, ("alice".into(), "shall we ship it?".into()));

    // A reaction, recorded at once without echo-message.
    client.react(buffer, "p1", "🚀", false).unwrap();
    assert_eq!(conn.recv().await, "@+reply=p1;+draft/react=🚀 TAGMSG #test");
    let reacted = |c: &Client| {
        c.state()
            .buffer(buffer)
            .unwrap()
            .lines()
            .iter()
            .find(|l| l.msgid.as_deref() == Some("p1"))
            .unwrap()
            .reactions
            .clone()
    };
    assert_eq!(reacted(&client)[0].nicks, ["me"]);
    client.react(buffer, "p1", "🚀", true).unwrap();
    assert_eq!(
        conn.recv().await,
        "@+reply=p1;+draft/unreact=🚀 TAGMSG #test"
    );
    assert!(reacted(&client).is_empty());
    client.quit(None).await;
}

#[tokio::test]
async fn nothing_is_sent_without_support_or_consent() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    // typing.send is off by default.
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "batch").await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send("@msgid=p1 :alice!a@h PRIVMSG #test :hi").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let mut conn = drive(&mut client, server).await;
    until(&mut client, |c| {
        channel(c).is_some_and(|b| c.state().last_message_from(b, "alice").is_some())
    })
    .await;
    let id = client.state().networks().next().unwrap().id;
    let buffer = channel(&client).unwrap();
    client.typing(id, "#test", true);
    nothing_sent(&client, &mut conn, "typing.send is off").await;
    // Without message-tags: no reactions, and a reply is plain text.
    let error = client.react(buffer, "p1", "👍", false).unwrap_err();
    assert!(error.to_string().contains("doesn't support"), "{error}");
    client.reply(id, "#test", "p1", "alice: hello").unwrap();
    assert_eq!(conn.recv().await, "PRIVMSG #test :alice: hello");
    // Several lines without draft/multiline: a message each.
    client.say(id, "#test", "one\ntwo", false).unwrap();
    assert_eq!(conn.recv().await, "PRIVMSG #test one");
    assert_eq!(conn.recv().await, "PRIVMSG #test two");
    client.quit(None).await;
}

#[tokio::test]
async fn multiline_messages_go_as_batches() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", dir.path());
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register(
            "me",
            "batch message-tags echo-message labeled-response draft/multiline=max-bytes=4096,max-lines=3",
        )
        .await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send(":srv 366 me #test :End").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let mut conn = drive(&mut client, server).await;
    until(&mut client, |c| channel(c).is_some()).await;
    let id = client.state().networks().next().unwrap().id;
    let buffer = channel(&client).unwrap();

    // Four lines, at most three a batch: two batches, each with its label.
    client
        .say(id, "#test", "one\n\nthree\nfour\n", false)
        .unwrap();
    let mut wire = Vec::new();
    for _ in 0..8 {
        wire.push(conn.recv().await);
    }
    assert_eq!(
        wire,
        [
            "@label=irsty1 BATCH +irsty1 draft/multiline #test",
            "@batch=irsty1 PRIVMSG #test one",
            "@batch=irsty1 PRIVMSG #test :",
            "@batch=irsty1 PRIVMSG #test three",
            "BATCH -irsty1",
            "@label=irsty2 BATCH +irsty2 draft/multiline #test",
            "@batch=irsty2 PRIVMSG #test four",
            "BATCH -irsty2",
        ]
    );
    let pending = |c: &Client| {
        c.state()
            .buffer(buffer)
            .unwrap()
            .lines()
            .iter()
            .filter(|l| matches!(l.delivery, Delivery::Pending { .. }))
            .count()
    };
    assert_eq!(pending(&client), 4);

    // The server echoes the first batch: its three lines are confirmed.
    for line in [
        "@label=irsty1;msgid=m1 BATCH +e1 draft/multiline #test",
        "@batch=e1 :me!u@h PRIVMSG #test one",
        "@batch=e1 :me!u@h PRIVMSG #test :",
        "@batch=e1 :me!u@h PRIVMSG #test three",
        "BATCH -e1",
    ] {
        conn.send(line).await;
    }
    until(&mut client, |c| pending(c) == 1).await;
    let lines: Vec<_> = client
        .state()
        .buffer(buffer)
        .unwrap()
        .lines()
        .iter()
        .cloned()
        .collect();
    let first = lines
        .iter()
        .position(|l| l.text.to_plain() == "one")
        .unwrap();
    assert_eq!(lines[first].msgid.as_deref(), Some("m1"));
    assert!(lines[first + 1].continued && lines[first + 2].continued);
    client.quit(None).await;
}
