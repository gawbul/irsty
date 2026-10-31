//! Turning terminal events into actions: keys (irssi defaults), paste, mouse,
//! and commands typed into the input line.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use irsty_core::Client;
use irsty_core::event::NetworkId;
use irsty_core::state::{BufferKind, LineKind};
use irsty_proto::Message;
use ratatui::layout::Position;

use crate::command::{self, Command};
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
    match command::parse(&line) {
        Ok(cmd) => execute(cmd, client, ui),
        Err(error) => {
            feedback(client, ui, LineKind::Error, &error.to_string());
            Flow::Continue
        }
    }
}

/// Writes a line into the active buffer.
fn feedback(client: &mut Client, ui: &Ui, kind: LineKind, text: &str) {
    if let Some(buffer) = ui.active {
        client.note_buffer(buffer, kind, text);
    }
}

/// Resolves a network by name, or the active one.
fn network(client: &Client, ui: &Ui, name: Option<&str>) -> Option<NetworkId> {
    match name {
        Some(name) => client
            .state()
            .networks()
            .find(|n| n.name.eq_ignore_ascii_case(name))
            .map(|n| n.id),
        None => ui.network(client.state()),
    }
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

#[allow(
    clippy::too_many_lines,
    reason = "one arm per command reads better than indirection"
)]
fn run(cmd: Command, client: &mut Client, ui: &mut Ui) -> Result<Flow, String> {
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
            client.send(id, msg).map_err(|e| e.to_string())?;
            let first = channels.split(',').next().unwrap_or_default().to_owned();
            ui.pending_switch = Some((id, first));
        }
        Command::Part { channel, reason } => {
            let id = need_net()?;
            let current = ui
                .active
                .and_then(|b| client.state().buffer(b))
                .filter(|b| b.kind == BufferKind::Channel)
                .map(|b| b.name.clone());
            let channel = channel
                .or(current)
                .ok_or("Not in a channel; use /part <#channel>")?;
            let mut msg = Message::new("PART").param(channel);
            if let Some(reason) = reason {
                msg = msg.param(reason);
            }
            client.send(id, msg).map_err(|e| e.to_string())?;
        }
        Command::Msg { target, text } => {
            let id = need_net()?;
            client
                .say(id, &target, &text, false)
                .map_err(|e| e.to_string())?;
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
        Command::Nick(nick) => {
            let id = need_net()?;
            client
                .send(id, Message::new("NICK").param(nick))
                .map_err(|e| e.to_string())?;
        }
        Command::Raw(line) => {
            let id = need_net()?;
            client.raw(id, &line).map_err(|e| e.to_string())?;
        }
        Command::Connect(name) => {
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
        Command::Close(reason) => {
            let buffer = ui.active.ok_or("Nothing to close")?;
            client
                .close_buffer(buffer, reason.as_deref())
                .map_err(|e| e.to_string())?;
            ui.on_update(client.state());
        }
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
