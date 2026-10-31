//! Terminal setup and restoration.
//!
//! The terminal is restored on drop, on panic (via a chained hook), and around
//! suspend (Ctrl+Z or SIGTSTP). Restoring is idempotent and ignores errors, so
//! it is safe to call from a panic.

use std::io::{self, Write};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Whether the terminal is currently in our mode.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HOOK: Once = Once::new();

/// Puts the terminal into raw mode on the alternate screen, and restores it
/// when dropped.
#[derive(Debug)]
pub struct TerminalGuard {
    mouse: bool,
}

impl TerminalGuard {
    /// Enters raw mode and the alternate screen, with bracketed paste and focus
    /// events, and mouse capture if `mouse` is set.
    pub fn enter(mouse: bool) -> io::Result<Self> {
        install_panic_hook();
        let guard = Self { mouse };
        guard.setup()?;
        Ok(guard)
    }

    fn setup(&self) -> io::Result<()> {
        enable_raw_mode()?;
        ACTIVE.store(true, Ordering::SeqCst);
        let mut out = io::stdout();
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        if self.mouse {
            execute!(out, EnableMouseCapture)?;
        }
        Ok(())
    }

    /// Restores the terminal, stops the process until it is continued (as Ctrl+Z
    /// would in a normal program), then sets the terminal up again. The caller
    /// must redraw everything afterwards.
    #[cfg(unix)]
    pub fn suspend(&self) -> io::Result<()> {
        restore();
        // SIGSTOP cannot be caught, so this always stops; it returns on SIGCONT.
        signal_hook::low_level::raise(signal_hook::consts::SIGSTOP)?;
        self.setup()
    }

    /// Suspending is a Unix feature; elsewhere this does nothing.
    #[cfg(not(unix))]
    pub fn suspend(&self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

/// Leaves the alternate screen and raw mode. Safe to call repeatedly.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
    let _ = execute!(
        out,
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
    let _ = out.flush();
}

/// Restores the terminal before the default panic message is printed, so the
/// message is readable and the shell is usable.
fn install_panic_hook() {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_is_idempotent_when_inactive() {
        // Never entered, so this must not touch the terminal or fail.
        restore();
        restore();
        assert!(!ACTIVE.load(Ordering::SeqCst));
    }
}
