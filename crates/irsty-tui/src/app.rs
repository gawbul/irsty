//! Turning terminal events into actions: keys (through the keymap), paste,
//! mouse and focus, and running the commands typed into the input line.

use std::fmt::Write as _;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use irsty_core::Client;
use irsty_core::config::{Alias, Config, IgnoreKind, IgnoreRule, SaslMechanism};
use irsty_core::config_file::{ConfigFile, EditError, SETTINGS};
use irsty_core::event::NetworkId;
use irsty_core::state::{Awaiting, BufferId, BufferKind, LineKind};
use irsty_proto::{Message, ctcp};
use ratatui::layout::Position;

use crate::command::{self, Command, ServerCommand, Window};
use crate::complete;
use crate::keymap::{Action, Chord, Format, Keymap};
use crate::ui::{Completion, Switcher, Ui};

/// What the run loop should do after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Keep going; redraw.
    Continue,
    /// Clear and redraw everything (Ctrl+L, resize).
    Redraw,
    /// Suspend the process (Ctrl+Z).
    Suspend,
    /// Quit every network with this reason, then exit.
    Quit(String),
}

/// Default quit message.
pub const QUIT_MESSAGE: &str = "Leaving";

/// Handles one terminal event.
pub fn handle_event(event: Event, client: &mut Client, ui: &mut Ui) -> Flow {
    match event {
        Event::Key(key) if key.is_press() || key.is_repeat() => handle_key(key, client, ui),
        Event::Paste(text) => {
            on_paste(&text, ui);
            Flow::Continue
        }
        Event::Mouse(mouse) => handle_mouse(mouse, client, ui),
        Event::Resize(..) => Flow::Redraw,
        Event::FocusGained => {
            ui.focused = true;
            Flow::Continue
        }
        Event::FocusLost => {
            ui.focused = false;
            Flow::Continue
        }
        // Key releases.
        Event::Key(_) => Flow::Continue,
    }
}

/// A paste of one line goes into the input; several lines wait for the user
/// to say whether to send them as separate messages.
fn on_paste(text: &str, ui: &mut Ui) {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<String> = text
        .trim_end_matches('\n')
        .split('\n')
        .map(str::to_owned)
        .collect();
    if lines.len() > 1 {
        ui.paste = Some(lines);
    } else {
        ui.input.insert(&text);
    }
}

fn handle_key(key: KeyEvent, client: &mut Client, ui: &mut Ui) -> Flow {
    if ui.paste.is_some() {
        return paste_key(key, client, ui);
    }
    if ui.switcher.is_some() {
        switcher_key(key, client, ui);
        return Flow::Continue;
    }
    if std::mem::take(&mut ui.format_prefix) {
        if let KeyCode::Char(c) = key.code
            && let Some(format) = Format::from_weechat_letter(c)
        {
            ui.input.insert(&format.code().to_string());
        }
        return Flow::Continue;
    }
    if let Some(digits) = ui.jump.as_mut() {
        if let KeyCode::Char(c @ '0'..='9') = key.code {
            digits.push(c);
            if digits.len() == 2 {
                let number = digits.parse().unwrap_or(0);
                ui.jump = None;
                ui.switch_to_number(client.state(), number);
            }
            return Flow::Continue;
        }
        ui.jump = None;
    }
    let action = ui.keymap.action(&key);
    if !matches!(action, Some(Action::Complete | Action::CompleteBack)) {
        ui.completion = None;
    }
    if let Some(action) = action {
        return run_action(action, client, ui);
    }
    // Unbound keys type their character, unless Ctrl or Alt is held.
    let typed = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
    if let (KeyCode::Char(c), true) = (key.code, typed) {
        let mut buf = [0u8; 4];
        ui.input.insert(c.encode_utf8(&mut buf));
    }
    Flow::Continue
}

fn run_action(action: Action, client: &mut Client, ui: &mut Ui) -> Flow {
    let state = client.state();
    match action {
        Action::Send => return submit(client, ui),
        Action::Suspend => return Flow::Suspend,
        Action::Redraw => return Flow::Redraw,
        Action::Complete => complete(client, ui, true),
        Action::CompleteBack => complete(client, ui, false),
        Action::Switcher => ui.switcher = Some(Switcher::default()),
        Action::NextBuffer => ui.cycle(state, true),
        Action::PrevBuffer => ui.cycle(state, false),
        Action::NextActive => ui.next_active(state),
        Action::Back => ui.go_back(state),
        Action::Forward => ui.go_forward(state),
        Action::Buffer(n) => ui.switch_to_number(state, usize::from(n)),
        Action::Jump => ui.jump = Some(String::new()),
        Action::PageUp | Action::ScrollUp | Action::ScrollTop => {
            match action {
                Action::PageUp => ui.page(state, true),
                Action::ScrollUp => ui.scroll_by(state, 3),
                _ => ui.scroll_by(state, isize::MAX / 2),
            }
            // Past the oldest line: ask the server for older history.
            if ui.at_oldest(client.state())
                && let Some(buffer) = ui.active
            {
                client.request_older(buffer);
            }
        }
        Action::PageDown => ui.page(state, false),
        Action::ScrollDown => ui.scroll_by(state, -3),
        Action::ScrollBottom => ui.scroll_by(state, isize::MIN / 2),
        Action::HistoryPrev => ui.input.history_prev(),
        Action::HistoryNext => ui.input.history_next(),
        Action::Left => ui.input.left(),
        Action::Right => ui.input.right(),
        Action::WordLeft => ui.input.word_left(),
        Action::WordRight => ui.input.word_right(),
        Action::Home => ui.input.home(),
        Action::End => ui.input.end(),
        Action::Backspace => ui.input.backspace(),
        Action::Delete => ui.input.delete(),
        Action::KillWordBack => ui.input.kill_word_back(),
        Action::KillWordForward => ui.input.kill_word_forward(),
        Action::KillToStart => ui.input.kill_to_start(),
        Action::KillToEnd => ui.input.kill_to_end(),
        Action::Yank => ui.input.yank(),
        Action::Transpose => ui.input.transpose(),
        Action::Format(format) => ui.input.insert(&format.code().to_string()),
        Action::FormatPrefix => ui.format_prefix = true,
        Action::None => {}
    }
    Flow::Continue
}

/// Tab completion: the first press finds matches, later presses cycle
/// through them (backwards with Shift+Tab).
fn complete(client: &Client, ui: &mut Ui, forward: bool) {
    let continuing = ui
        .completion
        .as_ref()
        .is_some_and(|c| c.text == ui.input.text());
    if !continuing {
        let (start, word) = ui.input.word_before_cursor();
        let aliases: Vec<&str> = ui.config.aliases.keys().map(String::as_str).collect();
        let matches = complete::complete(
            ui.input.text(),
            start,
            word,
            &complete::Context {
                state: client.state(),
                buffer: ui.active,
                aliases: &aliases,
            },
        );
        if matches.words.is_empty() {
            ui.completion = None;
            return;
        }
        ui.completion = Some(Completion {
            start,
            words: matches.words,
            suffix: matches.suffix,
            index: None,
            text: String::new(),
        });
    }
    let Some(completion) = ui.completion.as_mut() else {
        return;
    };
    let count = completion.words.len();
    let index = match (completion.index, forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(i), true) => (i + 1) % count,
        (Some(i), false) => (i + count - 1) % count,
    };
    completion.index = Some(index);
    let word = completion
        .words
        .get(index)
        .map(|w| format!("{w}{}", completion.suffix))
        .unwrap_or_default();
    let start = completion.start;
    ui.input.replace_before_cursor(start, &word);
    if let Some(completion) = ui.completion.as_mut() {
        ui.input.text().clone_into(&mut completion.text);
    }
}

fn switcher_key(key: KeyEvent, client: &Client, ui: &mut Ui) {
    let matches = ui.switcher_matches(client.state());
    let Some(switcher) = ui.switcher.as_mut() else {
        return;
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => ui.switcher = None,
        KeyCode::Char('g' | 'c') if ctrl => ui.switcher = None,
        KeyCode::Enter => {
            let chosen = matches.get(switcher.selected).map(|&(_, id, _)| id);
            ui.switcher = None;
            if let Some(id) = chosen {
                ui.switch(id);
            }
        }
        KeyCode::Up => switcher.selected = switcher.selected.saturating_sub(1),
        KeyCode::Char('p') if ctrl => switcher.selected = switcher.selected.saturating_sub(1),
        KeyCode::Down | KeyCode::Tab => {
            switcher.selected = (switcher.selected + 1).min(matches.len().saturating_sub(1));
        }
        KeyCode::Char('n') if ctrl => {
            switcher.selected = (switcher.selected + 1).min(matches.len().saturating_sub(1));
        }
        KeyCode::Backspace => {
            switcher.query.pop();
            switcher.selected = 0;
        }
        KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
            switcher.query.push(c);
            switcher.selected = 0;
        }
        _ => {}
    }
}

/// The multi-line paste prompt: Enter (or Y) sends each line as a message,
/// Esc (or N) cancels, and J joins the lines into the input instead.
fn paste_key(key: KeyEvent, client: &mut Client, ui: &mut Ui) -> Flow {
    let Some(lines) = ui.paste.take() else {
        return Flow::Continue;
    };
    match key.code {
        KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
            for line in lines.iter().filter(|l| !l.trim().is_empty()) {
                // Pasted text is never run as commands.
                let flow = execute(Command::Say(line.clone()), client, ui);
                if flow != Flow::Continue {
                    return flow;
                }
            }
        }
        KeyCode::Char('j' | 'J') => ui.input.insert(&lines.join(" ")),
        KeyCode::Esc | KeyCode::Char('n' | 'N') => {}
        // Anything else leaves the prompt up.
        _ => ui.paste = Some(lines),
    }
    Flow::Continue
}

fn handle_mouse(mouse: MouseEvent, client: &Client, ui: &mut Ui) -> Flow {
    let state = client.state();
    match mouse.kind {
        MouseEventKind::ScrollUp => ui.scroll_by(state, 3),
        MouseEventKind::ScrollDown => ui.scroll_by(state, -3),
        MouseEventKind::Down(MouseButton::Left) => {
            let (area, order) = &ui.buffer_list;
            if area.contains(Position::new(mouse.column, mouse.row)) {
                // The list scrolls to keep the active buffer visible; account for that.
                let height = usize::from(area.height);
                let active = ui
                    .active
                    .and_then(|a| order.iter().position(|&b| b == a))
                    .unwrap_or(0);
                let skip = (active + 1).saturating_sub(height);
                let row = usize::from(mouse.row - area.y) + skip;
                if let Some(&id) = order.get(row) {
                    ui.switch(id);
                }
            }
        }
        _ => {}
    }
    Flow::Continue
}

fn submit(client: &mut Client, ui: &mut Ui) -> Flow {
    let line = ui.input.submit();
    if line.is_empty() {
        return Flow::Continue;
    }
    // Typing scrolls back to the newest lines, as in irssi.
    let state = client.state();
    ui.scroll_by(state, isize::MIN / 2);
    run_line(&line, client, ui, 0)
}

/// How deep aliases may call aliases.
const MAX_ALIAS_DEPTH: usize = 8;

/// Runs one typed line: an alias, a command or text.
fn run_line(line: &str, client: &mut Client, ui: &mut Ui, depth: usize) -> Flow {
    if let Some(expanded) = expand_alias(line, client, ui) {
        if depth >= MAX_ALIAS_DEPTH {
            feedback(
                client,
                ui,
                LineKind::Error,
                "Aliases nest too deeply; stopping",
            );
            return Flow::Continue;
        }
        for command in expanded {
            let flow = run_line(&command, client, ui, depth + 1);
            if flow != Flow::Continue {
                return flow;
            }
        }
        return Flow::Continue;
    }
    match command::parse(line) {
        Ok(cmd) => execute(cmd, client, ui),
        Err(error) => {
            feedback(client, ui, LineKind::Error, &error.to_string());
            Flow::Continue
        }
    }
}

/// If `line` runs an alias (built-in commands win), its commands.
fn expand_alias(line: &str, client: &Client, ui: &Ui) -> Option<Vec<String>> {
    let body = line.strip_prefix('/').filter(|b| !b.starts_with('/'))?;
    let (name, args) = body.split_once(' ').unwrap_or((body, ""));
    if command::canonical(name).is_some() {
        return None;
    }
    let alias = ui.config.aliases.get(&name.to_lowercase())?;
    let state = client.state();
    let net = ui.network(state).and_then(|id| state.network(id));
    let channel = ui
        .target(state)
        .map(|(_, t)| t.to_owned())
        .unwrap_or_default();
    let nick = net.map(|n| n.nick.clone()).unwrap_or_default();
    let network = net.map(|n| n.name.clone()).unwrap_or_default();
    Some(command::expand(
        alias.commands(),
        args,
        &[
            ("nick", nick.as_str()),
            ("channel", channel.as_str()),
            ("network", network.as_str()),
        ],
    ))
}

/// Writes a line into the active buffer, or the status area when there is
/// no buffer yet.
pub fn feedback(client: &mut Client, ui: &mut Ui, kind: LineKind, text: &str) {
    if let Some(buffer) = ui.active {
        client.note_buffer(buffer, kind, text);
    } else {
        if ui.status.len() >= 200 {
            ui.status.remove(0);
        }
        ui.status.push((kind, text.to_owned()));
    }
}

/// Resolves a network by name, or the active one.
fn network(client: &Client, ui: &Ui, name: Option<&str>) -> Option<NetworkId> {
    match name {
        Some(name) => network_named(client, name),
        None => ui.network(client.state()),
    }
}

fn network_named(client: &Client, name: &str) -> Option<NetworkId> {
    client
        .state()
        .networks()
        .find(|n| n.name.eq_ignore_ascii_case(name))
        .map(|n| n.id)
}

/// Runs a parsed command.
pub fn execute(cmd: Command, client: &mut Client, ui: &mut Ui) -> Flow {
    let result = run(cmd, client, ui);
    match result {
        Ok(flow) => flow,
        Err(error) => {
            feedback(client, ui, LineKind::Error, &error);
            Flow::Continue
        }
    }
}

type Outcome = Result<Flow, String>;

fn send(client: &Client, id: NetworkId, msg: Message) -> Result<(), String> {
    client.send(id, msg).map_err(|e| e.to_string())
}

/// The current channel, for commands that act on one.
fn current_channel(client: &Client, ui: &Ui) -> Result<(NetworkId, String), String> {
    let state = client.state();
    ui.active
        .and_then(|b| state.buffer(b))
        .filter(|b| b.kind == BufferKind::Channel)
        .map(|b| (b.network, b.name.clone()))
        .ok_or_else(|| "Not in a channel; switch to one first".to_owned())
}

/// The channel given, or the current one.
fn channel_or_current(
    client: &Client,
    ui: &Ui,
    channel: Option<String>,
) -> Result<(NetworkId, String), String> {
    match channel {
        Some(channel) => {
            let id = ui
                .network(client.state())
                .ok_or("No network here; switch to a network's buffer first")?;
            Ok((id, channel))
        }
        None => current_channel(client, ui),
    }
}

/// The buffer a command's replies should go to.
fn here(ui: &Ui) -> Result<BufferId, String> {
    ui.active
        .ok_or_else(|| "No network yet; add one with /server add".to_owned())
}

/// Saves a config change and applies what can change live.
fn save(
    client: &mut Client,
    ui: &mut Ui,
    change: impl FnOnce(&mut ConfigFile) -> Result<Config, EditError>,
) -> Result<Config, String> {
    let file = ui
        .config_file
        .as_mut()
        .ok_or("There is no config file to change")?;
    let config = change(file).map_err(|e| e.to_string())?;
    client.apply_config(&config);
    ui.show_nicklist = config.ui.nicklist;
    ui.time_format.clone_from(&config.ui.timestamp_format);
    ui.keymap = Keymap::new(&config.ui.keymap, &config.keys).0;
    ui.nick_colours = crate::theme::NickColours::new(config.ui.nick_colours, &config.nick_colours);
    ui.config = config.clone();
    Ok(config)
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per command reads better than indirection"
)]
fn run(cmd: Command, client: &mut Client, ui: &mut Ui) -> Outcome {
    // `/window close` is `/close`.
    let cmd = match cmd {
        Command::Window(Window::Close) => Command::Close(None),
        other => other,
    };
    let net = ui.network(client.state());
    let need_net =
        || net.ok_or_else(|| "No network here; switch to a network's buffer first".to_owned());
    match cmd {
        Command::Say(text) => {
            if text.trim().is_empty() {
                return Ok(Flow::Continue);
            }
            let (id, target) = ui
                .target(client.state())
                .map(|(id, t)| (id, t.to_owned()))
                .ok_or("This is a server buffer: use /msg or /join")?;
            client
                .say(id, &target, &text, false)
                .map_err(|e| e.to_string())?;
        }
        Command::Me(text) => {
            let (id, target) = ui
                .target(client.state())
                .map(|(id, t)| (id, t.to_owned()))
                .ok_or("/me needs a channel or query")?;
            client
                .say(id, &target, &text, true)
                .map_err(|e| e.to_string())?;
        }
        Command::Join { channels, keys } => {
            let id = need_net()?;
            let mut msg = Message::new("JOIN").param(channels.clone());
            if let Some(keys) = keys {
                msg = msg.param(keys);
            }
            send(client, id, msg)?;
            let first = channels.split(',').next().unwrap_or_default().to_owned();
            ui.pending_switch = Some((id, first));
        }
        Command::Part { channel, reason } => {
            let (id, channel) = channel_or_current(client, ui, channel)
                .map_err(|_| "Not in a channel; use /part <#channel>".to_owned())?;
            let mut msg = Message::new("PART").param(channel);
            if let Some(reason) = reason {
                msg = msg.param(reason);
            }
            send(client, id, msg)?;
        }
        Command::Msg { target, text } => {
            let id = need_net()?;
            client
                .say(id, &target, &text, false)
                .map_err(|e| e.to_string())?;
        }
        Command::Notice { target, text } => {
            let id = need_net()?;
            send(
                client,
                id,
                Message::new("NOTICE").param(&target).param(&text),
            )?;
            // With echo-message the server's copy shows it.
            let echoes = client
                .state()
                .network(id)
                .is_some_and(|n| n.has_cap("echo-message"));
            if !echoes {
                feedback(
                    client,
                    ui,
                    LineKind::Notice,
                    &format!("-> -{target}- {text}"),
                );
            }
        }
        Command::Ctcp {
            target,
            command,
            args,
        } => {
            let id = need_net()?;
            let body = ctcp::encode(&command, args.as_deref());
            send(
                client,
                id,
                Message::new("PRIVMSG").param(&target).param(body),
            )?;
            feedback(
                client,
                ui,
                LineKind::Info,
                &format!("CTCP {command} sent to {target}"),
            );
        }
        Command::Query { nick, text } => {
            let id = need_net()?;
            if let Some(buffer) = client.open_query(id, &nick) {
                ui.switch(buffer);
            }
            if let Some(text) = text {
                client
                    .say(id, &nick, &text, false)
                    .map_err(|e| e.to_string())?;
            }
        }
        Command::Nick(nick) => send(client, need_net()?, Message::new("NICK").param(nick))?,
        Command::Whois(args) => {
            let id = need_net()?;
            let words: Vec<&str> = args.split_whitespace().collect();
            // "/whois nick nick" asks the user's own server, which knows idle time.
            let nick = words.last().copied().unwrap_or_default().to_owned();
            client.expect_reply(id, Awaiting::Whois(nick), here(ui)?);
            let msg = words.iter().fold(Message::new("WHOIS"), |m, w| m.param(*w));
            send(client, id, msg)?;
        }
        Command::Whowas(nick) => {
            let id = need_net()?;
            client.expect_reply(id, Awaiting::Whowas(nick.clone()), here(ui)?);
            send(client, id, Message::new("WHOWAS").param(nick))?;
        }
        Command::Who(mask) => {
            let (id, mask) = match mask {
                Some(mask) => (need_net()?, mask),
                None => current_channel(client, ui)
                    .map_err(|_| "Usage: /who <mask> (or use it in a channel)".to_owned())?,
            };
            client.expect_reply(id, Awaiting::Who(mask.clone()), here(ui)?);
            send(client, id, Message::new("WHO").param(mask))?;
        }
        Command::Names(channel) => {
            let (id, channel) = channel_or_current(client, ui, channel)?;
            client.expect_reply(id, Awaiting::Names(channel.clone()), here(ui)?);
            send(client, id, Message::new("NAMES").param(channel))?;
        }
        Command::Topic { channel, text } => {
            let (id, channel) = channel_or_current(client, ui, channel)?;
            if let Some(text) = text {
                send(client, id, Message::new("TOPIC").param(channel).param(text))?;
            } else {
                let known = client
                    .state()
                    .network(id)
                    .and_then(|n| n.channel(&channel))
                    .and_then(|c| {
                        c.topic
                            .as_ref()
                            .map(irsty_proto::format::StyledText::to_plain)
                    });
                match known {
                    Some(topic) => feedback(
                        client,
                        ui,
                        LineKind::Topic,
                        &format!("Topic for {channel}: {topic}"),
                    ),
                    None => send(client, id, Message::new("TOPIC").param(channel))?,
                }
            }
        }
        Command::Mode { target, modes } => {
            let (id, target) = match target {
                Some(target) => (need_net()?, target),
                None => current_channel(client, ui)?,
            };
            let msg = modes
                .split_whitespace()
                .fold(Message::new("MODE").param(target), Message::param);
            send(client, id, msg)?;
        }
        Command::Members { mode, give, nicks } => {
            let (id, channel) = current_channel(client, ui)?;
            // ISUPPORT MODES says how many modes fit in one MODE; 3 is safe.
            let per = client
                .state()
                .network(id)
                .and_then(|n| n.isupport.raw.get("MODES"))
                .and_then(|m| m.parse::<usize>().ok())
                .unwrap_or(3)
                .clamp(1, 12);
            for chunk in nicks.chunks(per) {
                let sign = if give { '+' } else { '-' };
                let modes = format!("{sign}{}", mode.to_string().repeat(chunk.len()));
                let msg = chunk
                    .iter()
                    .fold(Message::new("MODE").param(&channel).param(modes), |m, n| {
                        m.param(n)
                    });
                send(client, id, msg)?;
            }
        }
        Command::Kick { nick, reason } => {
            let (id, channel) = current_channel(client, ui)?;
            let mut msg = Message::new("KICK").param(channel).param(nick);
            if let Some(reason) = reason {
                msg = msg.param(reason);
            }
            send(client, id, msg)?;
        }
        Command::Ban(None) => {
            let (id, channel) = current_channel(client, ui)?;
            send(client, id, Message::new("MODE").param(channel).param("+b"))?;
        }
        Command::Ban(Some(target)) => {
            let (id, channel) = current_channel(client, ui)?;
            let mask = if target.contains(['!', '@', '*']) {
                target
            } else {
                // Ban the host when we know it, which survives nick changes.
                client
                    .state()
                    .network(id)
                    .and_then(|n| n.user(&target))
                    .and_then(|u| u.userhost.as_deref())
                    .and_then(|uh| uh.split_once('@'))
                    .map_or_else(
                        || format!("{target}!*@*"),
                        |(_, host)| format!("*!*@{host}"),
                    )
            };
            send(
                client,
                id,
                Message::new("MODE").param(channel).param("+b").param(mask),
            )?;
        }
        Command::Unban(mask) => {
            let (id, channel) = current_channel(client, ui)?;
            send(
                client,
                id,
                Message::new("MODE").param(channel).param("-b").param(mask),
            )?;
        }
        Command::Invite { nick, channel } => {
            let (id, channel) = channel_or_current(client, ui, channel)?;
            send(
                client,
                id,
                Message::new("INVITE").param(nick).param(channel),
            )?;
        }
        Command::Away(message) => {
            let msg =
                message.map_or_else(|| Message::new("AWAY"), |m| Message::new("AWAY").param(m));
            send(client, need_net()?, msg)?;
        }
        Command::List(args) => {
            let id = need_net()?;
            client.expect_reply(id, Awaiting::List, here(ui)?);
            let msg = args
                .unwrap_or_default()
                .split_whitespace()
                .fold(Message::new("LIST"), Message::param);
            send(client, id, msg)?;
        }
        Command::Raw(line) => {
            let id = need_net()?;
            client.raw(id, &line).map_err(|e| e.to_string())?;
        }
        Command::Connect(name) | Command::Server(ServerCommand::Connect(name)) => {
            let id = network(client, ui, name.as_deref()).ok_or("No such network")?;
            if client.is_running(id) {
                client.reconnect(id).map_err(|e| e.to_string())?;
            } else {
                client.connect(id).map_err(|e| e.to_string())?;
            }
        }
        Command::Disconnect {
            network: name,
            reason,
        } => {
            let id = network(client, ui, name.as_deref()).ok_or("No such network")?;
            client
                .disconnect(id, reason.as_deref())
                .map_err(|e| e.to_string())?;
        }
        Command::Server(server) => run_server(server, client, ui)?,
        Command::Close(reason) => {
            let buffer = ui.active.ok_or("Nothing to close")?;
            client
                .close_buffer(buffer, reason.as_deref())
                .map_err(|e| e.to_string())?;
            ui.on_update(client.state());
        }
        Command::Clear => client.clear_buffer(here(ui)?),
        Command::Url(choice) => run_url(choice, client, ui)?,
        Command::Window(window) => match window {
            Window::Number(n) => ui.switch_to_number(client.state(), n),
            Window::Next => ui.cycle(client.state(), true),
            Window::Prev => ui.cycle(client.state(), false),
            // `/window close` became `/close` above.
            Window::Close => {}
            Window::List => {
                let state = client.state();
                let lines: Vec<String> = ui
                    .order(state)
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &b)| {
                        let buffer = state.buffer(b)?;
                        let net = state.network(buffer.network)?;
                        Some(format!("{:>3}. {} ({})", i + 1, buffer.name, net.name))
                    })
                    .collect();
                for line in lines {
                    feedback(client, ui, LineKind::Info, &line);
                }
            }
        },
        Command::Ignore(None) => {
            let lines: Vec<String> = if ui.config.ignore.is_empty() {
                vec!["No ignores".into()]
            } else {
                ui.config.ignore.iter().map(describe_ignore).collect()
            };
            for line in lines {
                feedback(client, ui, LineKind::Info, &line);
            }
        }
        Command::Ignore(Some(rule)) => {
            let text = describe_ignore(&rule);
            save(client, ui, |f| f.add_ignore(&rule))?;
            feedback(client, ui, LineKind::Info, &format!("Ignoring {text}"));
        }
        Command::Unignore(mask) => {
            save(client, ui, |f| f.remove_ignore(&mask))?;
            feedback(
                client,
                ui,
                LineKind::Info,
                &format!("No longer ignoring {mask}"),
            );
        }
        Command::Highlight(None) => {
            let h = &ui.config.highlight;
            let lines: Vec<String> = if h.words.is_empty() && h.regex.is_empty() {
                vec!["No highlights besides your nick".into()]
            } else {
                h.words
                    .iter()
                    .map(|w| format!("Highlight word: {w}"))
                    .chain(h.regex.iter().map(|r| format!("Highlight regex: {r}")))
                    .collect()
            };
            for line in lines {
                feedback(client, ui, LineKind::Info, &line);
            }
        }
        Command::Highlight(Some((pattern, regex))) => {
            save(client, ui, |f| f.add_highlight(&pattern, regex))?;
            feedback(
                client,
                ui,
                LineKind::Info,
                &format!("Highlighting {pattern}"),
            );
        }
        Command::Unhighlight(pattern) => {
            save(client, ui, |f| f.remove_highlight(&pattern))?;
            feedback(
                client,
                ui,
                LineKind::Info,
                &format!("No longer highlighting {pattern}"),
            );
        }
        Command::Alias { name, commands } => run_alias(name, commands, client, ui)?,
        Command::Unalias(name) => {
            save(client, ui, |f| f.remove_alias(&name))?;
            feedback(
                client,
                ui,
                LineKind::Info,
                &format!("Removed alias /{name}"),
            );
        }
        Command::Set { key, value, reset } => run_set(key, value, reset, client, ui)?,
        Command::Bind { key, action, reset } => run_bind(key, action, reset, client, ui)?,
        Command::Unbind(key) => run_bind(Some(key), Some("none".into()), false, client, ui)?,
        Command::Quit(reason) => {
            return Ok(Flow::Quit(
                reason.unwrap_or_else(|| QUIT_MESSAGE.to_owned()),
            ));
        }
        Command::Help(name) => {
            for line in command::help(name.as_deref()).map_err(|e| e.to_string())? {
                feedback(client, ui, LineKind::Info, &line);
            }
        }
    }
    Ok(Flow::Continue)
}

fn describe_ignore(rule: &IgnoreRule) -> String {
    let mut text = rule.mask.clone();
    if rule.what.len() != IgnoreKind::ALL.len() {
        let what: Vec<&str> = rule.what.iter().map(|k| k.name()).collect();
        let _ = write!(text, " ({})", what.join(", "));
    }
    if !rule.networks.is_empty() {
        let _ = write!(text, " on {}", rule.networks.join(", "));
    }
    if !rule.channels.is_empty() {
        let _ = write!(text, " in {}", rule.channels.join(", "));
    }
    text
}

fn run_server(server: ServerCommand, client: &mut Client, ui: &mut Ui) -> Result<(), String> {
    match server {
        ServerCommand::Connect(_) => {}
        ServerCommand::Add {
            network,
            server,
            options,
        } => {
            let config = save(client, ui, |f| f.add_server(&network, &server, &options))?;
            let net = config
                .networks
                .iter()
                .find(|n| n.name.eq_ignore_ascii_case(&network))
                .ok_or("The network vanished from the config")?
                .clone();
            let path = saved_to(ui);
            if let Some(id) = network_named(client, &network) {
                client.update_network(id, &net);
                feedback(
                    client,
                    ui,
                    LineKind::Info,
                    &format!("Added {} to {network}; saved to {path}", server.host),
                );
            } else {
                let id = client.add_network(&net);
                if let Some(buffer) = client.state().network(id).map(|n| n.server_buffer) {
                    ui.switch(buffer);
                }
                feedback(
                    client,
                    ui,
                    LineKind::Info,
                    &format!("Added network {network}; saved to {path}. Connecting..."),
                );
            }
        }
        ServerCommand::Del { network, host } => {
            let config = save(client, ui, |f| f.remove(&network, host.as_deref()))?;
            let id = network_named(client, &network);
            match (host, id) {
                (None, Some(id)) => {
                    client.remove_network(id, Some("Network removed"));
                    ui.on_update(client.state());
                    feedback(
                        client,
                        ui,
                        LineKind::Info,
                        &format!("Removed network {network}"),
                    );
                }
                (Some(host), Some(id)) => {
                    if let Some(net) = config
                        .networks
                        .iter()
                        .find(|n| n.name.eq_ignore_ascii_case(&network))
                    {
                        client.update_network(id, net);
                    }
                    feedback(
                        client,
                        ui,
                        LineKind::Info,
                        &format!("Removed {host} from {network}"),
                    );
                }
                _ => feedback(
                    client,
                    ui,
                    LineKind::Info,
                    &format!("Removed {network} from the config"),
                ),
            }
        }
        ServerCommand::Update { network, options } => {
            let config = save(client, ui, |f| f.update(&network, &options))?;
            if let (Some(id), Some(net)) = (
                network_named(client, &network),
                config
                    .networks
                    .iter()
                    .find(|n| n.name.eq_ignore_ascii_case(&network)),
            ) {
                client.update_network(id, net);
            }
            feedback(
                client,
                ui,
                LineKind::Info,
                &format!("Updated {network}; reconnecting if connected"),
            );
        }
        ServerCommand::List => {
            let lines = server_list(&ui.config);
            for line in lines {
                feedback(client, ui, LineKind::Info, &line);
            }
        }
    }
    Ok(())
}

/// `/server list`: one line per configured network.
fn server_list(config: &Config) -> Vec<String> {
    if config.networks.is_empty() {
        vec!["No networks; add one with /server add (see /help server)".into()]
    } else {
        config
            .networks
            .iter()
            .map(|n| {
                let servers: Vec<String> = n.servers.iter().map(ToString::to_string).collect();
                let mut line = format!(
                    "{}: {} (nicks {})",
                    n.name,
                    servers.join(", "),
                    n.nicks.join(", ")
                );
                if !n.autojoin.is_empty() {
                    let _ = write!(line, ", autojoin {}", n.autojoin.join(","));
                }
                if let Some(sasl) = &n.sasl {
                    let mechanism = match sasl.mechanism {
                        SaslMechanism::Plain => "plain",
                        SaslMechanism::ScramSha256 => "scram-sha-256",
                        SaslMechanism::External => "external",
                    };
                    let _ = write!(line, ", SASL {mechanism}");
                }
                line
            })
            .collect()
    }
}

fn saved_to(ui: &Ui) -> String {
    ui.config_file
        .as_ref()
        .map(|f| f.path().display().to_string())
        .unwrap_or_default()
}

fn run_alias(
    name: Option<String>,
    commands: Option<Vec<String>>,
    client: &mut Client,
    ui: &mut Ui,
) -> Result<(), String> {
    let show = |name: &str, alias: &Alias| format!("/{name} = {}", alias.commands().join(" ; "));
    match (name, commands) {
        (None, _) => {
            let lines: Vec<String> = if ui.config.aliases.is_empty() {
                vec!["No aliases".into()]
            } else {
                ui.config.aliases.iter().map(|(n, a)| show(n, a)).collect()
            };
            for line in lines {
                feedback(client, ui, LineKind::Info, &line);
            }
        }
        (Some(name), None) => {
            let line = ui
                .config
                .aliases
                .get(&name)
                .map(|a| show(&name, a))
                .ok_or_else(|| format!("No alias named /{name}"))?;
            feedback(client, ui, LineKind::Info, &line);
        }
        (Some(name), Some(commands)) => {
            if command::canonical(&name).is_some() {
                return Err(format!("/{name} is a built-in command"));
            }
            save(client, ui, |f| f.set_alias(&name, &commands))?;
            feedback(client, ui, LineKind::Info, &format!("Alias /{name} saved"));
        }
    }
    Ok(())
}

fn run_bind(
    key: Option<String>,
    action: Option<String>,
    reset: bool,
    client: &mut Client,
    ui: &mut Ui,
) -> Result<(), String> {
    let Some(key) = key else {
        let lines: Vec<String> = ui
            .keymap
            .list()
            .into_iter()
            .map(|(k, a)| format!("{k:<14} {a}"))
            .collect();
        for line in lines {
            feedback(client, ui, LineKind::Info, &line);
        }
        return Ok(());
    };
    let chord = Chord::parse(&key)?;
    let name = chord.to_string();
    if reset {
        save(client, ui, |f| f.unbind(&name))?;
        let now = ui
            .keymap
            .get(&chord)
            .map_or_else(|| "nothing".into(), Action::name);
        feedback(
            client,
            ui,
            LineKind::Info,
            &format!("{name} is back to {now}"),
        );
        return Ok(());
    }
    match action {
        None => {
            let now = ui
                .keymap
                .get(&chord)
                .map_or_else(|| "nothing".into(), Action::name);
            feedback(client, ui, LineKind::Info, &format!("{name} = {now}"));
        }
        Some(action) => {
            let action = Action::from_name(&action)
                .ok_or_else(|| format!("No action named {action:?}; see docs/KEYBINDINGS.md"))?;
            save(client, ui, |f| f.bind(&name, &action.name()))?;
            feedback(
                client,
                ui,
                LineKind::Info,
                &format!("{name} = {}", action.name()),
            );
        }
    }
    Ok(())
}

/// Links listed by `/url`.
const URLS_LISTED: usize = 10;

fn run_url(choice: Option<usize>, client: &mut Client, ui: &mut Ui) -> Result<(), String> {
    let buffer = here(ui)?;
    // Newest first, each link once.
    let mut links: Vec<String> = Vec::new();
    if let Some(buf) = client.state().buffer(buffer) {
        for line in buf.lines().iter().rev() {
            let text = line.text.to_plain();
            for range in crate::urls::find(&text).into_iter().rev() {
                let link = text[range].to_owned();
                if !links.contains(&link) {
                    links.push(link);
                }
            }
            if links.len() >= URLS_LISTED {
                break;
            }
        }
    }
    links.truncate(URLS_LISTED);
    match choice {
        None if links.is_empty() => feedback(client, ui, LineKind::Info, "No links in this buffer"),
        None => {
            let listed: Vec<String> = links
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{:>3}. {l}", i + 1))
                .collect();
            for line in listed {
                feedback(client, ui, LineKind::Info, &line);
            }
        }
        Some(n) => {
            let link = n
                .checked_sub(1)
                .and_then(|i| links.get(i))
                .ok_or_else(|| format!("No link {n}; /url lists them"))?
                .clone();
            crate::urls::open(&link).map_err(|e| format!("Cannot open {link}: {e}"))?;
            feedback(client, ui, LineKind::Info, &format!("Opening {link}"));
        }
    }
    Ok(())
}

fn run_set(
    key: Option<String>,
    value: Option<String>,
    reset: bool,
    client: &mut Client,
    ui: &mut Ui,
) -> Result<(), String> {
    let current = |config: &Config, key: &str| -> String {
        match key {
            "scrollback" => config.scrollback.to_string(),
            "logging.enabled" => config.logging.enabled.to_string(),
            "logging.dir" => config
                .logging
                .dir
                .as_ref()
                .map_or_else(|| "(default)".into(), |d| d.display().to_string()),
            "ui.mouse" => config.ui.mouse.to_string(),
            "ui.nicklist" => config.ui.nicklist.to_string(),
            "ui.timestamp_format" => config.ui.timestamp_format.clone(),
            "ui.keymap" => config.ui.keymap.clone(),
            "ui.nick_colours" => config.ui.nick_colours.to_string(),
            "logging.rotate" => format!("{:?}", config.logging.rotate).to_lowercase(),
            "notifications.method" => format!("{:?}", config.notifications.method).to_lowercase(),
            "notifications.when_focused" => config.notifications.when_focused.to_string(),
            "notifications.show_text" => config.notifications.show_text.to_string(),
            "ctcp.replies" => config.ctcp.replies.to_string(),
            "ctcp.version" => config.ctcp.version.clone(),
            "ctcp.time" => config.ctcp.time.to_string(),
            _ => String::new(),
        }
    };
    let restart = |key: &str| key.starts_with("logging.") || key == "ui.mouse";
    match (key, value) {
        (None, _) => {
            let lines: Vec<String> = SETTINGS
                .iter()
                .map(|(k, _)| format!("{k} = {}", current(&ui.config, k)))
                .collect();
            for line in lines {
                feedback(client, ui, LineKind::Info, &line);
            }
        }
        (Some(key), _) if reset => {
            let config = save(client, ui, |f| f.unset(&key))?;
            let mut text = format!("{key} reset to {}", current(&config, &key));
            if restart(&key) {
                text.push_str(" (takes effect when Irsty restarts)");
            }
            feedback(client, ui, LineKind::Info, &text);
        }
        (Some(key), None) => {
            if !SETTINGS.iter().any(|(k, _)| *k == key) {
                return Err(format!("Unknown setting {key}; /set lists them"));
            }
            let line = format!("{key} = {}", current(&ui.config, &key));
            feedback(client, ui, LineKind::Info, &line);
        }
        (Some(key), Some(value)) => {
            let config = save(client, ui, |f| f.set(&key, &value))?;
            let mut text = format!("{key} = {}", current(&config, &key));
            if restart(&key) {
                text.push_str(" (takes effect when Irsty restarts)");
            }
            feedback(client, ui, LineKind::Info, &text);
        }
    }
    Ok(())
}
