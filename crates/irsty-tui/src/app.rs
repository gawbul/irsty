//! Turning terminal events into actions: keys (irssi defaults), paste, mouse,
//! and commands typed into the input line.

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
use crate::ui::Ui;

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
            ui.input.insert(&text);
            Flow::Continue
        }
        Event::Mouse(mouse) => handle_mouse(mouse, client, ui),
        Event::Resize(..) => Flow::Redraw,
        _ => Flow::Continue,
    }
}

/// irssi's Alt+key window numbers: 1 to 0 for windows 1 to 10, then q to p for
/// 11 to 20.
fn window_number(c: char) -> Option<usize> {
    if let Some(d) = c.to_digit(10) {
        return Some(if d == 0 {
            10
        } else {
            usize::try_from(d).unwrap_or(0)
        });
    }
    "qwertyuiop".find(c).map(|i| i + 11)
}

fn handle_key(key: KeyEvent, client: &mut Client, ui: &mut Ui) -> Flow {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let state = client.state();
    match key.code {
        KeyCode::Enter => return submit(client, ui),
        KeyCode::Char('z') if ctrl => return Flow::Suspend,
        KeyCode::Char('l') if ctrl => return Flow::Redraw,
        KeyCode::Char('n') if ctrl => ui.cycle(state, true),
        KeyCode::Char('p') if ctrl => ui.cycle(state, false),
        KeyCode::Right if alt => ui.cycle(state, true),
        KeyCode::Left if alt => ui.cycle(state, false),
        KeyCode::Char('a') if alt => ui.next_active(state),
        KeyCode::Char(c) if alt && window_number(c).is_some() => {
            ui.switch_to_number(state, window_number(c).unwrap_or(1));
        }
        KeyCode::PageUp => ui.page(state, true),
        KeyCode::PageDown => ui.page(state, false),
        KeyCode::End if ctrl => ui.scroll_by(state, isize::MIN / 2),
        KeyCode::Up => ui.input.history_prev(),
        KeyCode::Down => ui.input.history_next(),
        KeyCode::Left if ctrl => ui.input.word_left(),
        KeyCode::Right if ctrl => ui.input.word_right(),
        KeyCode::Left => ui.input.left(),
        KeyCode::Right => ui.input.right(),
        KeyCode::Home => ui.input.home(),
        KeyCode::End => ui.input.end(),
        KeyCode::Backspace if alt => ui.input.kill_word_back(),
        KeyCode::Backspace => ui.input.backspace(),
        KeyCode::Delete => ui.input.delete(),
        KeyCode::Char('a') if ctrl => ui.input.home(),
        KeyCode::Char('e') if ctrl => ui.input.end(),
        KeyCode::Char('b') if ctrl => ui.input.left(),
        KeyCode::Char('f') if ctrl => ui.input.right(),
        KeyCode::Char('d') if ctrl => ui.input.delete(),
        KeyCode::Char('h') if ctrl => ui.input.backspace(),
        KeyCode::Char('k') if ctrl => ui.input.kill_to_end(),
        KeyCode::Char('u') if ctrl => ui.input.kill_to_start(),
        KeyCode::Char('w') if ctrl => ui.input.kill_word_back(),
        KeyCode::Char('y') if ctrl => ui.input.yank(),
        KeyCode::Char('b') if alt => ui.input.word_left(),
        KeyCode::Char('f') if alt => ui.input.word_right(),
        KeyCode::Char('d') if alt => ui.input.kill_word_forward(),
        // Other control and alt chords are unbound for now (M7 makes keys configurable).
        KeyCode::Char(_) if ctrl || alt => {}
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            ui.input.insert(c.encode_utf8(&mut buf));
        }
        _ => {}
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
fn feedback(client: &mut Client, ui: &mut Ui, kind: LineKind, text: &str) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn irssi_window_numbers() {
        assert_eq!(window_number('1'), Some(1));
        assert_eq!(window_number('0'), Some(10));
        assert_eq!(window_number('q'), Some(11));
        assert_eq!(window_number('p'), Some(20));
        assert_eq!(window_number('z'), None);
    }
}
