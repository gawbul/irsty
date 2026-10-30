//! Headless mode: connect and print every buffer's new lines to stdout.
//!
//! Useful for scripting, debugging and as a smoke test. Only sanitised text is
//! printed, so it is as safe in a terminal as the TUI.

use std::collections::HashMap;
use std::io::Write;

use color_eyre::eyre::Result;
use irsty_core::Client;
use irsty_core::Config;
use irsty_core::config::Paths;
use irsty_core::state::{BufferId, Line, LineKind, State};
use irsty_proto::format::sanitise_identifier;
use jiff::tz::TimeZone;

/// Runs until interrupted, then quits every network cleanly.
pub(crate) async fn run(config: &Config, paths: Option<&Paths>) -> Result<()> {
    let mut client = Client::start(config, paths, None).await;
    let mut printed: HashMap<BufferId, u64> = HashMap::new();
    let all: Vec<BufferId> = client.state().buffers().map(|b| b.id).collect();
    print_new(client.state(), &all, &mut printed)?;

    let mut terminate = terminate_signal()?;
    loop {
        tokio::select! {
            changed = client.next() => match changed {
                Some(buffers) => print_new(client.state(), &buffers, &mut printed)?,
                None => break,
            },
            _ = tokio::signal::ctrl_c() => break,
            () = &mut terminate => break,
        }
    }
    client.quit(Some("Leaving")).await;
    let all: Vec<BufferId> = client.state().buffers().map(|b| b.id).collect();
    print_new(client.state(), &all, &mut printed)
}

#[cfg(unix)]
fn terminate_signal() -> Result<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>> {
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    Ok(Box::pin(async move {
        signal.recv().await;
    }))
}

#[cfg(not(unix))]
fn terminate_signal() -> Result<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>> {
    Ok(Box::pin(std::future::pending()))
}

/// Prints lines added to `buffers` since the last call.
fn print_new(
    state: &State,
    buffers: &[BufferId],
    printed: &mut HashMap<BufferId, u64>,
) -> Result<()> {
    let tz = TimeZone::system();
    let mut out = std::io::stdout().lock();
    for &id in buffers {
        let Some(buffer) = state.buffer(id) else {
            continue;
        };
        let seen = printed.insert(id, buffer.total).unwrap_or(0);
        let new = usize::try_from(buffer.total.saturating_sub(seen)).unwrap_or(usize::MAX);
        let network = state
            .network(buffer.network)
            .map_or("?", |n| n.name.as_str());
        let lines = buffer.lines();
        for line in lines.iter().skip(lines.len().saturating_sub(new)) {
            let time = line.time.to_zoned(tz.clone()).strftime("%H:%M:%S");
            let label = format!(
                "{}/{}",
                sanitise_identifier(network),
                sanitise_identifier(&buffer.name)
            );
            let result = writeln!(out, "{time} [{label}] {}", render(line));
            match result {
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
                other => other?,
            }
        }
    }
    Ok(())
}

fn render(line: &Line) -> String {
    let sender = line.sender.as_deref().unwrap_or("");
    let text = line.text.to_plain();
    match line.kind {
        LineKind::Message => format!("<{sender}> {text}"),
        LineKind::Action => format!("* {sender} {text}"),
        LineKind::Notice => format!("-{sender}- {text}"),
        LineKind::Error => format!("!!! {text}"),
        _ => format!("-!- {text}"),
    }
}
