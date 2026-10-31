//! IRCv3 extensions end to end: a scripted mock server offers capabilities and
//! sends what a real server would, and the tests check the client's state.

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
use irsty_core::event::NetworkId;
use irsty_core::state::{BufferId, Delivery, LineKind, User};
use support::*;

/// Connects with `caps` offered (and acknowledged), joins `#test` with
/// `alice` and `bob` in it, and returns the client and the mock's end.
async fn connected(caps: &str, extra: &str) -> (Client, Conn, tempfile::TempDir) {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), extra, dir.path());
    let caps = caps.to_owned();
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", &caps).await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send(":srv 353 me = #test :@me alice!a@alice.host bob!b@bob.host")
            .await;
        c.send(":srv 366 me #test :End of /NAMES list").await;
        c
    });
    let mut client = Client::start(&cfg, None, Some(fast(Duration::from_secs(30))));
    let conn = drive(&mut client, server).await;
    until(&mut client, |c| member_count(c) == 3).await;
    (client, conn, dir)
}

fn net_id(client: &Client) -> NetworkId {
    client.state().networks().next().unwrap().id
}

fn member_count(client: &Client) -> usize {
    let net = client.state().networks().next().unwrap();
    net.channel("#test").map_or(0, |c| c.members().count())
}

fn user(client: &Client, nick: &str) -> Option<User> {
    client
        .state()
        .networks()
        .next()
        .unwrap()
        .user(nick)
        .cloned()
}

fn channel_buffer(client: &Client) -> BufferId {
    let net = client.state().networks().next().unwrap();
    net.channel("#test").unwrap().buffer
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

/// Sends `lines` from the server, then drives the client until `done`.
async fn feed(
    client: &mut Client,
    conn: &mut Conn,
    lines: &[&str],
    done: impl FnMut(&Client) -> bool,
) {
    for line in lines {
        conn.send(line).await;
    }
    until(client, done).await;
}

#[tokio::test]
async fn account_tag_account_notify_and_extended_join() {
    let (mut client, mut conn, _dir) =
        connected("account-tag account-notify extended-join", "").await;
    feed(
        &mut client,
        &mut conn,
        &[":carol!c@carol.host JOIN #test carols_account :Carol Real"],
        |c| user(c, "carol").is_some(),
    )
    .await;
    let carol = user(&client, "carol").unwrap();
    assert_eq!(carol.account.as_deref(), Some("carols_account"));
    assert_eq!(carol.realname.as_deref(), Some("Carol Real"));
    assert_eq!(carol.userhost.as_deref(), Some("c@carol.host"));

    // account-notify: logging out, then account-tag on a message.
    feed(
        &mut client,
        &mut conn,
        &[":carol!c@carol.host ACCOUNT *"],
        |c| user(c, "carol").unwrap().account.is_none(),
    )
    .await;
    feed(
        &mut client,
        &mut conn,
        &["@account=other :carol!c@carol.host PRIVMSG #test :hi"],
        |c| user(c, "carol").unwrap().account.as_deref() == Some("other"),
    )
    .await;
    // An extended JOIN with "*" means no account.
    feed(
        &mut client,
        &mut conn,
        &[":dave!d@h JOIN #test * :Dave"],
        |c| user(c, "dave").is_some(),
    )
    .await;
    assert_eq!(user(&client, "dave").unwrap().account, None);
    // Users we no longer share a channel with are forgotten.
    feed(&mut client, &mut conn, &[":dave!d@h PART #test"], |c| {
        user(c, "dave").is_none()
    })
    .await;
    client.quit(None).await;
}

#[tokio::test]
async fn away_notify() {
    let (mut client, mut conn, _dir) = connected("away-notify", "").await;
    feed(&mut client, &mut conn, &[":alice!a@h AWAY :lunch"], |c| {
        user(c, "alice").unwrap().away.as_deref() == Some("lunch")
    })
    .await;
    feed(&mut client, &mut conn, &[":alice!a@h AWAY"], |c| {
        user(c, "alice").unwrap().away.is_none()
    })
    .await;
    client.quit(None).await;
}

#[tokio::test]
async fn chghost_and_setname() {
    let (mut client, mut conn, _dir) = connected("chghost setname", "").await;
    feed(
        &mut client,
        &mut conn,
        &[
            ":alice!a@alice.host CHGHOST newuser cloak.example",
            ":alice!newuser@cloak.example SETNAME :Alice Liddell",
        ],
        |c| user(c, "alice").unwrap().realname.is_some(),
    )
    .await;
    let alice = user(&client, "alice").unwrap();
    assert_eq!(alice.userhost.as_deref(), Some("newuser@cloak.example"));
    assert_eq!(alice.realname.as_deref(), Some("Alice Liddell"));
    // Neither is announced: they replace noisy QUIT and JOIN pairs.
    assert!(
        !texts(&client, channel_buffer(&client))
            .iter()
            .any(|l| l.contains("cloak"))
    );
    client.quit(None).await;
}

#[tokio::test]
async fn invite_notify() {
    let (mut client, mut conn, _dir) = connected("invite-notify", "").await;
    let channel = channel_buffer(&client);
    feed(
        &mut client,
        &mut conn,
        &[":op!o@h INVITE carol #test"],
        |c| {
            texts(c, channel)
                .iter()
                .any(|l| l == "op invited carol to #test")
        },
    )
    .await;
    feed(
        &mut client,
        &mut conn,
        &[":op!o@h INVITE me #secret"],
        |c| {
            server_text(c)
                .iter()
                .any(|l| l == "op invited you to #secret")
        },
    )
    .await;
    client.quit(None).await;
}

#[tokio::test]
async fn batch_netsplit_and_netjoin() {
    let (mut client, mut conn, _dir) = connected("batch", "").await;
    let channel = channel_buffer(&client);
    feed(
        &mut client,
        &mut conn,
        &[
            "BATCH +split netsplit hub.example leaf.example",
            "@batch=split :alice!a@h QUIT :hub.example leaf.example",
            "@batch=split :bob!b@h QUIT :hub.example leaf.example",
        ],
        // Held until the batch ends.
        |c| member_count(c) == 3,
    )
    .await;
    feed(&mut client, &mut conn, &["BATCH -split"], |c| {
        member_count(c) == 1
    })
    .await;
    let lines = texts(&client, channel);
    assert_eq!(
        lines.last().unwrap(),
        "Netsplit hub.example <-> leaf.example: alice, bob (2)"
    );
    assert!(!lines.iter().any(|l| l.contains("has quit")));
    assert!(user(&client, "alice").is_none());

    feed(
        &mut client,
        &mut conn,
        &[
            "BATCH +join netjoin hub.example leaf.example",
            "@batch=join :alice!a@h JOIN #test",
            "@batch=join :bob!b@h JOIN #test",
            "BATCH -join",
        ],
        |c| member_count(c) == 3,
    )
    .await;
    assert_eq!(
        texts(&client, channel).last().unwrap(),
        "Netjoin hub.example <-> leaf.example: alice, bob (2)"
    );
    // Other batch types are handled in order when they end.
    feed(
        &mut client,
        &mut conn,
        &[
            "BATCH +x example.com/unknown",
            "@batch=x :alice!a@h PRIVMSG #test :held",
            "BATCH -x",
        ],
        |c| texts(c, channel).last().is_some_and(|l| l == "held"),
    )
    .await;
    client.quit(None).await;
}

/// echo-message with labeled-response: our line is pending until the
/// labelled echo confirms it (once, not twice), and a labelled error marks
/// exactly that line as failed.
#[tokio::test]
async fn echo_message_with_labels() {
    let (mut client, mut conn, dir) =
        connected("echo-message labeled-response batch message-tags", "").await;
    let id = net_id(&client);
    let channel = channel_buffer(&client);
    let delivery = |c: &Client, text: &str| {
        c.state()
            .buffer(channel)
            .unwrap()
            .lines()
            .iter()
            .filter(|l| l.text.to_plain() == text)
            .map(|l| l.delivery.clone())
            .collect::<Vec<_>>()
    };

    client.say(id, "#test", "hello", false).unwrap();
    conn.expect("@label=irsty1 PRIVMSG #test hello").await;
    assert_eq!(
        delivery(&client, "hello"),
        [Delivery::Pending {
            label: Some("irsty1".into())
        }]
    );
    feed(
        &mut client,
        &mut conn,
        &["@label=irsty1;time=2026-01-02T03:04:05.000Z :me!u@h PRIVMSG #test :hello"],
        |c| delivery(c, "hello") == [Delivery::Delivered],
    )
    .await;
    let line = client
        .state()
        .buffer(channel)
        .unwrap()
        .lines()
        .iter()
        .find(|l| l.text.to_plain() == "hello")
        .unwrap()
        .clone();
    assert_eq!(line.time.to_string(), "2026-01-02T03:04:05Z");
    assert_eq!(line.kind, LineKind::Message);

    // A labelled error fails that line.
    client.say(id, "#test", "second", false).unwrap();
    conn.expect("@label=irsty2 PRIVMSG #test second").await;
    feed(
        &mut client,
        &mut conn,
        &["@label=irsty2 :srv 404 me #test :Cannot send to channel"],
        |c| matches!(delivery(c, "second").as_slice(), [Delivery::Failed { .. }]),
    )
    .await;
    assert_eq!(
        delivery(&client, "second"),
        [Delivery::Failed {
            reason: "Cannot send to channel".into()
        }]
    );
    // The error shows where we typed, not in the server buffer.
    assert!(
        texts(&client, channel)
            .iter()
            .any(|l| l == "#test Cannot send to channel")
    );

    // A labelled batch carrying the echo.
    client.say(id, "#test", "third", false).unwrap();
    conn.expect("@label=irsty3 PRIVMSG #test third").await;
    feed(
        &mut client,
        &mut conn,
        &[
            "@label=irsty3 BATCH +r labeled-response",
            "@batch=r :me!u@h PRIVMSG #test :third",
            "BATCH -r",
        ],
        |c| delivery(c, "third") == [Delivery::Delivered],
    )
    .await;
    client.quit(None).await;

    // Only confirmed lines reach the log, each once.
    let log = std::fs::read_to_string(dir.path().join("mock").join("#test.log")).unwrap();
    assert_eq!(log.matches("<me> hello").count(), 1, "{log}");
    assert_eq!(log.matches("<me> third").count(), 1, "{log}");
    assert!(!log.contains("second"), "{log}");
}

/// echo-message without labels: echoes confirm by text, oldest first; lines
/// still pending at a disconnect are marked as not delivered; a services
/// password we typed stays redacted when it comes back.
#[tokio::test]
async fn echo_message_without_labels() {
    let (mut client, mut conn, _dir) = connected("echo-message", "").await;
    let id = net_id(&client);
    let channel = channel_buffer(&client);
    let pending = |c: &Client| {
        c.state()
            .buffer(channel)
            .unwrap()
            .lines()
            .iter()
            .filter(|l| matches!(l.delivery, Delivery::Pending { .. }))
            .count()
    };
    client.say(id, "#test", "same", false).unwrap();
    client.say(id, "#test", "same", false).unwrap();
    client.say(id, "#test", "other", false).unwrap();
    for _ in 0..3 {
        conn.recv().await;
    }
    assert_eq!(pending(&client), 3);
    feed(
        &mut client,
        &mut conn,
        &[":me!u@h PRIVMSG #test :same"],
        |c| pending(c) == 2,
    )
    .await;
    // The oldest "same" was confirmed; the second still waits.
    let lines = client.state().buffer(channel).unwrap().lines().clone();
    let same: Vec<_> = lines.iter().filter(|l| l.raw == "same").collect();
    assert_eq!(same.len(), 2);
    assert_eq!(same[0].delivery, Delivery::Delivered);
    assert!(matches!(
        same[1].delivery,
        Delivery::Pending { label: None }
    ));

    client
        .say(id, "NickServ", "IDENTIFY hunter2", false)
        .unwrap();
    conn.expect("PRIVMSG NickServ :IDENTIFY hunter2").await;
    let query = client.state().find_buffer(id, "NickServ").unwrap();
    feed(
        &mut client,
        &mut conn,
        &[":me!u@h PRIVMSG NickServ :IDENTIFY hunter2"],
        |c| {
            c.state()
                .buffer(query)
                .unwrap()
                .lines()
                .iter()
                .all(|l| l.delivery == Delivery::Delivered)
        },
    )
    .await;
    assert!(
        !texts(&client, query).iter().any(|l| l.contains("hunter2")),
        "{:?}",
        texts(&client, query)
    );

    // Drop the connection: what is still pending was never delivered.
    drop(conn);
    until(&mut client, |c| pending(c) == 0).await;
    let failed: Vec<_> = client
        .state()
        .buffer(channel)
        .unwrap()
        .lines()
        .iter()
        .filter(|l| matches!(l.delivery, Delivery::Failed { .. }))
        .map(|l| l.raw.clone())
        .collect();
    assert_eq!(failed, ["same", "other"]);
    client.quit(None).await;
}

#[tokio::test]
async fn standard_replies() {
    let (mut client, mut conn, _dir) = connected("standard-replies", "").await;
    feed(
        &mut client,
        &mut conn,
        &[
            "FAIL JOIN CHANNEL_NAME_TOO_LONG #averyveryverylongname :Channel name is too long",
            "WARN REHASH CERTS_EXPIRED :Certificate [xyz] has expired",
            "NOTE * OPERATOR_MESSAGE :The message",
        ],
        |c| server_text(c).len() >= 3 && server_text(c).last().unwrap().contains("The message"),
    )
    .await;
    let lines = server_text(&client);
    let n = lines.len();
    assert_eq!(
        lines[n - 3..],
        [
            "JOIN failed: Channel name is too long (CHANNEL_NAME_TOO_LONG)",
            "REHASH warning: Certificate [xyz] has expired (CERTS_EXPIRED)",
            "Server note: The message (OPERATOR_MESSAGE)",
        ]
    );
    let net = client.state().networks().next().unwrap();
    let last_three: Vec<LineKind> = client
        .state()
        .buffer(net.server_buffer)
        .unwrap()
        .lines()
        .iter()
        .rev()
        .take(3)
        .map(|l| l.kind)
        .collect();
    assert_eq!(
        last_three,
        [LineKind::Info, LineKind::Info, LineKind::Error]
    );
    client.quit(None).await;
}

#[tokio::test]
async fn monitor_watches_nicks() {
    let (mut client, mut conn, _dir) = connected("", "monitor = [\"friend\", \"foe\"]").await;
    conn.send(":srv 005 me MONITOR=100 :are supported by this server")
        .await;
    // The client must keep running to act on the 005.
    let server = tokio::spawn(async move {
        conn.expect("MONITOR + friend,foe").await;
        conn
    });
    let mut conn = drive(&mut client, server).await;
    feed(
        &mut client,
        &mut conn,
        &[":srv 730 me :friend!f@h", ":srv 731 me :foe"],
        |c| server_text(c).iter().any(|l| l == "friend is online"),
    )
    .await;
    // An initial "offline" isn't news; going offline after being online is.
    assert!(!server_text(&client).iter().any(|l| l.contains("foe")));
    feed(&mut client, &mut conn, &[":srv 731 me :friend"], |c| {
        server_text(c).iter().any(|l| l == "friend is offline")
    })
    .await;
    let net = client.state().networks().next().unwrap();
    assert_eq!(net.watches[0].online, Some(false));
    assert_eq!(net.watches[1].online, Some(false));
    client.quit(None).await;
}

#[tokio::test]
async fn monitor_unsupported_is_reported() {
    let (mut client, mut conn, _dir) = connected("", "monitor = [\"friend\"]").await;
    feed(
        &mut client,
        &mut conn,
        &[":srv 376 me :End of /MOTD command."],
        |c| {
            server_text(c)
                .iter()
                .any(|l| l.contains("doesn't support MONITOR"))
        },
    )
    .await;
    client.quit(None).await;
}

/// cap-notify: a capability offered later is requested then.
#[tokio::test]
async fn cap_notify_requests_new_capabilities() {
    let (mut client, mut conn, _dir) = connected("cap-notify", "").await;
    conn.send(":srv CAP me NEW :away-notify draft/unknown")
        .await;
    conn.expect("CAP REQ away-notify").await;
    conn.send(":srv CAP me ACK :away-notify").await;
    until(&mut client, |c| {
        c.state().networks().next().unwrap().has_cap("away-notify")
    })
    .await;
    client.quit(None).await;
}

/// multi-prefix and userhost-in-names: every mode and the host from NAMES.
#[tokio::test]
async fn multi_prefix_and_userhost_in_names() {
    let (mut client, mut conn, _dir) = connected("multi-prefix userhost-in-names", "").await;
    feed(
        &mut client,
        &mut conn,
        &[
            ":srv 005 me PREFIX=(ov)@+ :are supported",
            ":srv 353 me = #test :@+carol!c@carol.host",
            ":srv 366 me #test :End of /NAMES list",
        ],
        |c| user(c, "carol").is_some(),
    )
    .await;
    let net = client.state().networks().next().unwrap();
    let carol = net
        .channel("#test")
        .unwrap()
        .member(net.isupport.casemapping, "carol")
        .unwrap();
    assert_eq!(carol.modes, "ov");
    assert_eq!(
        user(&client, "carol").unwrap().userhost.as_deref(),
        Some("c@carol.host")
    );
    client.quit(None).await;
}
