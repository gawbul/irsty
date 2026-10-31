//! The UI event loop (ADR 0002): client events, terminal input, signals and a
//! clock feed one loop that redraws only when something changed, at most about
//! 60 times a second. Nothing here blocks on the network.

use std::io;
use std::time::Duration;

use crossterm::event::EventStream;
use futures::StreamExt;
use irsty_core::Client;
use irsty_core::config::Config;
use irsty_core::config_file::ConfigFile;
use jiff::Timestamp;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep_until};

use crate::app::{self, Flow};
use crate::terminal::TerminalGuard;
use crate::ui::Ui;
use crate::view;

/// Shortest time between frames.
const FRAME: Duration = Duration::from_millis(16);

type Term = Terminal<CrosstermBackend<io::Stdout>>;

/// Clears the screen and returns a terminal with empty buffers, so the next
/// draw repaints everything. `Terminal::clear` would instead query the cursor
/// position, which fails after two seconds on terminals that do not answer.
fn fresh_terminal() -> io::Result<Term> {
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Signals that end or pause the UI.
#[cfg(unix)]
struct Signals {
    terminate: tokio::signal::unix::Signal,
    hangup: tokio::signal::unix::Signal,
    stop: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    fn new() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            hangup: signal(SignalKind::hangup())?,
            stop: signal(SignalKind::from_raw(signal_hook::consts::SIGTSTP))?,
        })
    }

    async fn next(&mut self) -> Flow {
        tokio::select! {
            _ = self.terminate.recv() => Flow::Quit(app::QUIT_MESSAGE.to_owned()),
            _ = self.hangup.recv() => Flow::Quit("Terminal closed".to_owned()),
            _ = self.stop.recv() => Flow::Suspend,
        }
    }
}

#[cfg(not(unix))]
struct Signals;

#[cfg(not(unix))]
impl Signals {
    fn new() -> io::Result<Self> {
        Ok(Self)
    }

    async fn next(&mut self) -> Flow {
        std::future::pending().await
    }
}

/// Runs the terminal UI until the user quits, then quits every network.
/// `file` lets commands such as `/server` and `/set` save changes.
pub async fn run(mut client: Client, config: Config, file: Option<ConfigFile>) -> io::Result<()> {
    let guard = TerminalGuard::enter(config.ui.mouse)?;
    let mut terminal = fresh_terminal()?;
    let mut ui = Ui::new(&config.ui, client.state());
    ui.config = config;
    ui.config_file = file;
    let mut events = EventStream::new();
    let mut signals = Signals::new()?;
    let mut clock = interval(Duration::from_secs(1));
    clock.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut dirty = true;
    let mut next_frame = Instant::now();
    let mut client_running = true;

    let reason = loop {
        if dirty && Instant::now() >= next_frame {
            ui.now = Timestamp::now();
            terminal.draw(|frame| view::render(frame, client.state(), &mut ui))?;
            if let Some(active) = ui.active {
                client.mark_read(active);
            }
            dirty = false;
            next_frame = Instant::now() + FRAME;
        }

        let flow = tokio::select! {
            batch = client.next(), if client_running => {
                if batch.is_none() {
                    // Every network is down; keep the UI up for /connect.
                    client_running = false;
                }
                ui.on_update(client.state());
                Flow::Continue
            }
            event = events.next() => match event {
                Some(Ok(event)) => {
                    let flow = app::handle_event(event, &mut client, &mut ui);
                    ui.on_update(client.state());
                    // A command may have started a network again.
                    client_running = true;
                    flow
                }
                Some(Err(error)) => return Err(error),
                None => Flow::Quit(app::QUIT_MESSAGE.to_owned()),
            },
            flow = signals.next() => flow,
            _ = clock.tick() => Flow::Continue,
            () = sleep_until(next_frame), if dirty => continue,
        };

        dirty = true;
        match flow {
            Flow::Continue => {}
            Flow::Redraw => terminal = fresh_terminal()?,
            Flow::Suspend => {
                guard.suspend()?;
                terminal = fresh_terminal()?;
            }
            Flow::Quit(reason) => break reason,
        }
    };

    // Leave the terminal before waiting on the network, so a slow server never
    // leaves the user staring at a frozen UI.
    drop(terminal);
    drop(guard);
    client.quit(Some(&reason)).await;
    Ok(())
}
