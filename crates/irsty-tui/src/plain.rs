//! `irsty --plain`: a line-by-line interface for screen readers and braille
//! displays.
//!
//! Nothing is drawn: Irsty writes plain text lines to standard output as
//! things happen, and reads commands and messages a line at a time from
//! standard input, so the terminal's own line editing and the screen reader
//! work as they do in a shell. New lines in the current buffer are printed;
//! from other buffers, only mentions and private messages, with where they
//! came from. Switching buffers says where you are and reads out what you
//! hadn't seen. Commands, aliases and scripts work as in the full-screen UI.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, Write as _};

use irsty_core::Client;
use irsty_core::config::Config;
use irsty_core::config_file::ConfigFile;
use irsty_core::state::{Buffer, BufferId, BufferKind, Delivery, Line, LineKind, State};
use jiff::tz::TimeZone;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::app::{self, Flow};
use crate::ui::Ui;

/// Most unread lines read out when switching to a buffer.
pub const CATCH_UP: usize = 20;

/// What has been said so far: which lines of each buffer, and which status
/// messages, have been printed or passed over.
#[derive(Debug)]
pub struct Plain {
    seen: HashMap<BufferId, u64>,
    status: usize,
    active: Option<BufferId>,
    tz: TimeZone,
}

impl Plain {
    /// Starts with everything already in `state` counted as said.
    pub fn new(state: &State, ui: &Ui, tz: TimeZone) -> Self {
        Self {
            seen: state.buffers().map(|b| (b.id, b.total)).collect(),
            status: ui.status.len(),
            active: ui.active,
            tz,
        }
    }

    /// The lines to print for what has happened since the last call. Call it
    /// before the buffer on screen is marked read, so a switch can read out
    /// what was unread.
    pub fn output(&mut self, state: &State, ui: &Ui) -> Vec<String> {
        let mut out: Vec<String> = ui
            .status
            .iter()
            .skip(self.status)
            .map(|(_, text)| plain(text))
            .collect();
        self.status = ui.status.len();

        if ui.active != self.active {
            self.active = ui.active;
            if let Some(buffer) = ui.active.and_then(|id| state.buffer(id)) {
                out.push(format!("Now in {}.", place(state, buffer)));
                let unread = buffer.unread.min(CATCH_UP).min(buffer.lines().len());
                let skip = buffer.lines().len() - unread;
                out.extend(
                    buffer
                        .lines()
                        .iter()
                        .skip(skip)
                        .map(|l| self.describe(l, true)),
                );
                self.seen.insert(buffer.id, buffer.total);
            }
        }

        for buffer in state.buffers() {
            let seen = self.seen.insert(buffer.id, buffer.total).unwrap_or(0);
            let held = buffer.lines().len() as u64;
            let oldest = (buffer.total + 1).saturating_sub(held);
            let current = Some(buffer.id) == ui.active;
            for seq in seen.max(oldest.saturating_sub(1)) + 1..=buffer.total {
                let Some(line) = buffer.line(seq) else {
                    continue;
                };
                if current {
                    out.push(self.describe(line, line.history));
                } else if notable(state, buffer, line) {
                    out.push(format!(
                        "{}: {}",
                        place(state, buffer),
                        self.describe(line, false)
                    ));
                }
            }
        }
        self.seen.retain(|id, _| state.buffer(*id).is_some());
        out
    }

    /// A line as words: who and what, with its time only if asked (lines
    /// from history, where when matters).
    fn describe(&self, line: &Line, with_time: bool) -> String {
        let mut text = String::new();
        if with_time {
            let when = line.time.to_zoned(self.tz.clone()).strftime("%H:%M");
            {
                let _ = write!(text, "[{when}] ");
            };
        }
        let nick = line.sender.as_deref().unwrap_or("");
        if let Some(reply) = &line.reply {
            match &reply.parent {
                Some((parent, _)) => {
                    let _ = write!(text, "(replying to {parent}) ");
                }
                None => text.push_str("(replying) "),
            }
        }
        let body = line.text.to_plain();
        match line.kind {
            _ if line.continued => {
                let _ = write!(text, "  {body}");
            }
            LineKind::Message => {
                let _ = write!(text, "<{nick}> {body}");
            }
            LineKind::Action => {
                let _ = write!(text, "* {nick} {body}");
            }
            LineKind::Notice => {
                let _ = write!(text, "-{nick}- {body}");
            }
            LineKind::Error => {
                let _ = write!(text, "Error: {body}");
            }
            _ => text.push_str(&body),
        }
        if let Delivery::Failed { reason } = &line.delivery {
            {
                let _ = write!(text, " [not sent: {reason}]");
            };
        }
        plain(&text)
    }
}

/// Worth hearing from a buffer you aren't in: a live mention or private
/// message from someone else.
fn notable(state: &State, buffer: &Buffer, line: &Line) -> bool {
    let talk = matches!(
        line.kind,
        LineKind::Message | LineKind::Action | LineKind::Notice
    );
    let own = line
        .sender
        .as_deref()
        .is_some_and(|s| state.network(buffer.network).is_some_and(|n| n.is_me(s)));
    talk && !own && !line.history && (line.highlight || buffer.kind == BufferKind::Query)
}

/// `network/#channel`, or the network alone for its server buffer.
fn place(state: &State, buffer: &Buffer) -> String {
    let network = state
        .network(buffer.network)
        .map_or("", |n| n.name.as_str());
    let text = if buffer.kind == BufferKind::Server {
        network.to_owned()
    } else {
        format!("{network}/{}", buffer.name)
    };
    plain(&text)
}

/// Text safe to write to a terminal: no control characters at all.
fn plain(text: &str) -> String {
    irsty_proto::format::StyledText::plain(text)
        .to_plain()
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

/// The greeting, saying how to get around.
const INTRO: &str = "Irsty, plain mode. Type messages, or commands such as /join, /window list, \
/window 2 and /help. /quit, or Ctrl+D, leaves.";

/// Runs the plain interface until the user quits or input ends, then quits
/// every network.
pub async fn run(mut client: Client, config: Config, file: Option<ConfigFile>) -> io::Result<()> {
    let mut ui = Ui::new(&config.ui, client.state());
    let problems = ui.apply_config(&config);
    ui.config_file = file;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{INTRO}")?;
    for problem in problems {
        writeln!(stdout, "Error: {}", plain(&problem))?;
    }
    stdout.flush()?;
    drop(stdout);

    let mut plain_out = Plain::new(client.state(), &ui, TimeZone::system());
    let mut flow = app::load_scripts(&mut client, &mut ui);
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut client_running = true;
    let reason = loop {
        let mut lines = plain_out.output(client.state(), &ui);
        // Notifications are announced as lines. Mentions and private
        // messages already are; what is left (DCC offers, in a server
        // buffer) is said unless that buffer is the current one.
        for n in client.take_notifications() {
            let server = client
                .state()
                .buffer(n.buffer)
                .is_some_and(|b| b.kind == BufferKind::Server);
            if server && ui.active != Some(n.buffer) {
                lines.push(plain(&format!("{}: {}", n.network, n.text)));
            }
        }
        if !lines.is_empty() {
            let mut stdout = io::stdout().lock();
            for line in lines {
                writeln!(stdout, "{line}")?;
            }
            stdout.flush()?;
        }
        if let Some(active) = ui.active {
            client.mark_shown(&[active]);
        }
        if let Flow::Quit(reason) = std::mem::replace(&mut flow, Flow::Continue) {
            break reason;
        }
        let script_due = ui.scripts.next_due();
        flow = tokio::select! {
            batch = client.next(), if client_running => {
                if batch.is_none() {
                    client_running = false;
                }
                ui.on_update(client.state());
                app::script_lines(&mut client, &mut ui)
            }
            line = input.next_line() => match line? {
                Some(text) => {
                    let flow = app::input_line(&text, &mut client, &mut ui);
                    ui.on_update(client.state());
                    client_running = true;
                    flow
                }
                None => Flow::Quit(app::QUIT_MESSAGE.to_owned()),
            },
            _ = tokio::signal::ctrl_c() => Flow::Quit(app::QUIT_MESSAGE.to_owned()),
            () = tokio::time::sleep_until(
                script_due.map_or_else(tokio::time::Instant::now, tokio::time::Instant::from_std)
            ), if script_due.is_some() => app::script_timers(&mut client, &mut ui),
        };
    };
    client.quit(Some(&reason)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use irsty_core::config::UiConfig;
    use irsty_core::event::{NetEvent, NetworkId};
    use irsty_proto::Message;

    const NET: NetworkId = NetworkId(1);

    fn feed(s: &mut State, line: &str) {
        s.apply(NET, NetEvent::Message(Message::parse(line).unwrap()));
    }

    fn setup() -> (State, Ui, Plain) {
        let mut s = State::new(100);
        s.add_network(NET, "libera", "me", Vec::new());
        s.apply(NET, NetEvent::Registered { nick: "me".into() });
        feed(&mut s, ":me!u@h JOIN #a");
        feed(&mut s, ":me!u@h JOIN #b");
        let mut ui = Ui::new(&UiConfig::default(), &s);
        ui.switch(s.find_buffer(NET, "#a").unwrap());
        let mut plain = Plain::new(&s, &ui, TimeZone::UTC);
        // Nothing has happened yet.
        assert!(plain.output(&s, &ui).is_empty());
        (s, ui, plain)
    }

    #[test]
    fn says_what_happens_here_and_mentions_elsewhere() {
        let (mut s, ui, mut plain) = setup();
        feed(&mut s, ":alice!a@h PRIVMSG #a :hello all");
        feed(&mut s, ":bob!b@h PRIVMSG #a :\x01ACTION waves\x01");
        feed(&mut s, ":carol!c@h JOIN #a");
        feed(&mut s, ":dave!d@h PRIVMSG #b :chatter elsewhere");
        feed(&mut s, ":dave!d@h PRIVMSG #b :me: look at this");
        feed(&mut s, ":erin!e@h PRIVMSG me :psst");
        feed(&mut s, ":mallory!m@h PRIVMSG #a :\x1b[31mred\x07");
        assert_eq!(
            plain.output(&s, &ui),
            [
                "<alice> hello all",
                "* bob waves",
                "carol (c@h) has joined #a",
                "<mallory> ␛[31mred␇",
                "libera/#b: <dave> me: look at this",
                "libera/erin: <erin> psst",
            ]
        );
        assert!(plain.output(&s, &ui).is_empty(), "each line once");
    }

    #[test]
    fn switching_reads_out_what_was_unread() {
        let (mut s, mut ui, mut plain) = setup();
        let b = s.find_buffer(NET, "#b").unwrap();
        s.mark_read(b);
        feed(
            &mut s,
            "@time=2026-11-07T10:00:00Z :dave!d@h PRIVMSG #b :one",
        );
        feed(
            &mut s,
            "@time=2026-11-07T10:01:00Z :dave!d@h PRIVMSG #b :two",
        );
        assert!(plain.output(&s, &ui).is_empty(), "not a mention");
        ui.switch(b);
        assert_eq!(
            plain.output(&s, &ui),
            [
                "Now in libera/#b.",
                "[10:00] <dave> one",
                "[10:01] <dave> two"
            ]
        );
        s.mark_read(b);
        feed(&mut s, ":dave!d@h PRIVMSG #b :three");
        assert_eq!(plain.output(&s, &ui), ["<dave> three"]);
    }

    #[test]
    fn replies_failures_and_status() {
        let (mut s, mut ui, mut plain) = setup();
        feed(&mut s, "@msgid=p :alice!a@h PRIVMSG #a :question?");
        feed(&mut s, "@+reply=p :bob!b@h PRIVMSG #a :answer");
        let a = s.find_buffer(NET, "#a").unwrap();
        s.own_message(NET, "#a", LineKind::Message, "lost");
        s.fail_newest(a, 1, "send queue full");
        ui.status.push((LineKind::Info, "a status message".into()));
        let out = plain.output(&s, &ui);
        assert_eq!(
            out,
            [
                "a status message",
                "<alice> question?",
                "(replying to alice) <bob> answer",
                "<me> lost [not sent: send queue full]",
            ]
        );
    }
}
