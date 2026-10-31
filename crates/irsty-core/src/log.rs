//! Plain-text logs per buffer, irssi style: one greppable file per channel or
//! query, under `<dir>/<network>/<buffer>.log`.
//!
//! Writing happens on a dedicated thread behind a bounded queue. The app loop
//! never blocks on disk: if the queue is full, lines are counted as dropped and a
//! marker is written once there is room (ADR 0002).

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

#[cfg(test)]
use crate::state::Delivery;
use crate::state::{Line, LineKind};

/// Queue depth between the app loop and the writer thread.
const QUEUE: usize = 4096;

/// Open files kept before all are closed and reopened on demand.
const MAX_OPEN: usize = 64;

/// mIRC formatting codes, kept in logs so they can be replayed with colour.
const FORMATTING: [char; 9] = [
    '\x02', '\x03', '\x04', '\x0f', '\x11', '\x16', '\x1d', '\x1e', '\x1f',
];

struct Record {
    path: PathBuf,
    text: String,
}

/// Handle to the log writer thread.
#[derive(Debug)]
pub struct LogWriter {
    dir: PathBuf,
    tx: Option<SyncSender<Record>>,
    dropped: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl LogWriter {
    /// Starts the writer thread for logs under `dir`.
    pub fn spawn(dir: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped);
        let thread = std::thread::Builder::new()
            .name("irsty-log".into())
            .spawn(move || write_loop(&rx, &counter))?;
        Ok(Self {
            dir,
            tx: Some(tx),
            dropped,
            thread: Some(thread),
        })
    }

    /// The path a buffer's log is written to.
    pub fn path_for(&self, network: &str, buffer: &str) -> PathBuf {
        self.dir
            .join(file_name(network))
            .join(format!("{}.log", file_name(buffer)))
    }

    /// Queues a line. Never blocks.
    pub fn log(&self, network: &str, buffer: &str, line: &Line) {
        let Some(tx) = &self.tx else {
            return;
        };
        let record = Record {
            path: self.path_for(network, buffer),
            text: format_line(line),
        };
        match tx.try_send(record) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Flushes and stops the writer thread.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        self.stop();
    }
}

fn write_loop(rx: &Receiver<Record>, dropped: &AtomicU64) {
    let mut files: HashMap<PathBuf, BufWriter<File>> = HashMap::new();
    let mut failed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    while let Ok(first) = rx.recv() {
        let mut record = Some(first);
        while let Some(Record { path, text }) = record.take().or_else(|| rx.try_recv().ok()) {
            let lost = dropped.swap(0, Ordering::Relaxed);
            if files.len() >= MAX_OPEN && !files.contains_key(&path) {
                flush_all(&mut files);
                files.clear();
            }
            let file = match files.get_mut(&path) {
                Some(file) => file,
                None => match open(&path) {
                    Ok(file) => files.entry(path.clone()).or_insert(file),
                    Err(error) => {
                        if failed.insert(path.clone()) {
                            tracing::warn!(path = %path.display(), %error, "cannot open log file");
                        }
                        continue;
                    }
                },
            };
            let mut result = Ok(());
            if lost > 0 {
                result = writeln!(
                    file,
                    "-!- {lost} lines were not logged because the log writer fell behind"
                );
            }
            if let Err(error) = result.and_then(|()| writeln!(file, "{text}")) {
                tracing::warn!(path = %path.display(), %error, "cannot write log line");
            }
        }
        flush_all(&mut files);
    }
    flush_all(&mut files);
}

fn flush_all(files: &mut HashMap<PathBuf, BufWriter<File>>) {
    for (path, file) in files.iter_mut() {
        if let Err(error) = file.flush() {
            tracing::warn!(path = %path.display(), %error, "cannot flush log file");
        }
    }
}

fn open(path: &Path) -> std::io::Result<BufWriter<File>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Logs are private conversations.
        options.mode(0o600);
    }
    Ok(BufWriter::new(options.open(path)?))
}

/// Makes a network or buffer name safe as a single path component.
pub fn file_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .to_lowercase();
    if out.starts_with('.') || out.is_empty() {
        out.insert(0, '_');
    }
    // Keep well under common 255-byte name limits, on a character boundary.
    while out.len() > 100 {
        out.pop();
    }
    out
}

/// Formats a line for a log file: an ISO 8601 UTC timestamp, then irssi-style
/// text. mIRC formatting codes are kept; every other control character is made
/// visible, so viewing a log with `cat` is safe (ADR 0005).
pub fn format_line(line: &Line) -> String {
    let time = line.time.strftime("%Y-%m-%dT%H:%M:%SZ");
    let sender = line.sender.as_deref().unwrap_or("");
    let text = escape_for_log(&line.raw);
    match line.kind {
        LineKind::Message => format!("{time} <{sender}> {text}"),
        LineKind::Action => format!("{time} * {sender} {text}"),
        LineKind::Notice => format!("{time} -{sender}- {text}"),
        _ => format!("{time} -!- {text}"),
    }
}

fn escape_for_log(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            c if FORMATTING.contains(&c) => c,
            '\t' => ' ',
            '\0'..='\x1f' => {
                char::from_u32(0x2400 + u32::from(c)).unwrap_or(char::REPLACEMENT_CHARACTER)
            }
            '\x7f' => '\u{2421}',
            '\u{80}'..='\u{9f}' => char::REPLACEMENT_CHARACTER,
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use irsty_proto::format::StyledText;
    use jiff::Timestamp;

    fn line(kind: LineKind, sender: Option<&str>, raw: &str) -> Line {
        Line {
            time: "2026-10-30T12:34:56.789Z".parse::<Timestamp>().unwrap(),
            kind,
            sender: sender.map(str::to_owned),
            text: StyledText::parse(raw),
            raw: raw.to_owned(),
            delivery: Delivery::Delivered,
        }
    }

    #[test]
    fn formats_like_irssi() {
        assert_eq!(
            format_line(&line(LineKind::Message, Some("bob"), "hi")),
            "2026-10-30T12:34:56Z <bob> hi"
        );
        assert_eq!(
            format_line(&line(LineKind::Action, Some("bob"), "waves")),
            "2026-10-30T12:34:56Z * bob waves"
        );
        assert_eq!(
            format_line(&line(LineKind::Notice, Some("srv"), "note")),
            "2026-10-30T12:34:56Z -srv- note"
        );
        assert_eq!(
            format_line(&line(LineKind::Join, None, "bob joined")),
            "2026-10-30T12:34:56Z -!- bob joined"
        );
    }

    #[test]
    fn keeps_formatting_but_escapes_controls() {
        let out = format_line(&line(
            LineKind::Message,
            Some("x"),
            "\x02bold\x02 \x1b]0;t\x07\r\n\u{9b}",
        ));
        assert!(
            out.ends_with("\x02bold\x02 \u{241b}]0;t\u{2407}\u{240d}\u{240a}\u{fffd}"),
            "{out:?}"
        );
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("#Irsty"), "#irsty");
        assert_eq!(file_name("../etc/passwd"), "_.._etc_passwd");
        assert_eq!(file_name(""), "_");
        assert_eq!(file_name("a\0b:c"), "a_b_c");
        assert!(file_name(&"é".repeat(200)).len() <= 100);
    }

    #[test]
    fn writes_and_flushes_on_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LogWriter::spawn(dir.path().to_owned()).unwrap();
        writer.log(
            "Libera",
            "#Rust",
            &line(LineKind::Message, Some("a"), "one"),
        );
        writer.log(
            "Libera",
            "#Rust",
            &line(LineKind::Message, Some("b"), "two"),
        );
        let path = writer.path_for("Libera", "#Rust");
        assert!(path.ends_with("libera/#rust.log"));
        writer.shutdown();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("<b> two"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn many_files_do_not_exhaust_descriptors() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LogWriter::spawn(dir.path().to_owned()).unwrap();
        for i in 0..(MAX_OPEN * 2) {
            writer.log(
                "net",
                &format!("#c{i}"),
                &line(LineKind::Message, Some("a"), "x"),
            );
        }
        writer.shutdown();
        assert_eq!(
            fs::read_dir(dir.path().join("net")).unwrap().count(),
            MAX_OPEN * 2
        );
    }
}
