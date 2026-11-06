//! The search index: messages also written to the logs, kept in SQLite with
//! a full-text index (FTS5) so `/search` is quick over years of history.
//!
//! SQLite runs on a thread of its own. The app loop hands it lines and
//! queries over a bounded channel and never waits for it: a full queue drops
//! lines (they stay in the logs) rather than stall the UI. Answers come back
//! on a Tokio channel the client reads alongside network events.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::mpsc::UnboundedSender;

use crate::state::LineKind;

/// Lines and queries waiting for the index thread.
const QUEUE: usize = 4096;
/// Most lines written in one transaction.
const BATCH: usize = 512;
/// Most results a search may ask for.
pub const MAX_RESULTS: usize = 500;

/// A line to index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The network's name.
    pub network: String,
    /// The channel or nick, or `-server`.
    pub buffer: String,
    /// When it was said.
    pub time: Timestamp,
    /// What it is: a message, an action or a notice.
    pub kind: LineKind,
    /// Who said it (sanitised).
    pub sender: Option<String>,
    /// What they said, sanitised and without formatting.
    pub text: String,
    /// The server's id for it, so history seen twice is indexed once.
    pub msgid: Option<String>,
}

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Words, all of which must appear (as words or word prefixes).
    pub words: String,
    /// Only on this network.
    pub network: Option<String>,
    /// Only in this channel or query (needs `network`).
    pub buffer: Option<String>,
    /// Only from this nick.
    pub from: Option<String>,
    /// Most results, newest first before they are put in time order.
    pub limit: usize,
}

/// A line found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The network's name.
    pub network: String,
    /// The channel or nick.
    pub buffer: String,
    /// When it was said.
    pub time: Timestamp,
    /// Who said it.
    pub sender: Option<String>,
    /// What they said, with the matching words in bold (mIRC `\x02`).
    pub text: String,
}

/// The answer to a search: its id and the hits, oldest first.
pub type Answer = (u64, Result<Vec<Hit>, String>);

enum Job {
    Add(Entry),
    Search(u64, Query),
}

/// The index, running on its own thread until dropped.
#[derive(Debug)]
pub struct Index {
    jobs: SyncSender<Job>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add(_) => f.write_str("Add"),
            Self::Search(id, _) => write!(f, "Search({id})"),
        }
    }
}

impl Index {
    /// Opens (creating if needed) the index at `path` and starts its thread.
    /// Answers to searches go to `answers`.
    pub fn open(path: &Path, answers: UnboundedSender<Answer>) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        setup(&conn).map_err(|e| format!("{}: {e}", path.display()))?;
        let (jobs, rx) = mpsc::sync_channel(QUEUE);
        thread::Builder::new()
            .name("irsty-search".into())
            .spawn(move || run(&conn, &rx, &answers))
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs })
    }

    /// Queues a line. Returns false if the queue is full and it was dropped.
    pub fn add(&self, entry: Entry) -> bool {
        match self.jobs.try_send(Job::Add(entry)) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        }
    }

    /// Queues a search; its answer comes back tagged with `id`. Returns false
    /// if the queue is full.
    pub fn search(&self, id: u64, query: Query) -> bool {
        self.jobs.try_send(Job::Search(id, query)).is_ok()
    }
}

fn setup(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS lines (
             id INTEGER PRIMARY KEY,
             network TEXT NOT NULL,
             buffer TEXT NOT NULL COLLATE NOCASE,
             time INTEGER NOT NULL,
             kind TEXT NOT NULL,
             sender TEXT COLLATE NOCASE,
             text TEXT NOT NULL,
             msgid TEXT
         );
         CREATE INDEX IF NOT EXISTS lines_place ON lines (network, buffer, time);
         CREATE UNIQUE INDEX IF NOT EXISTS lines_msgid
             ON lines (network, buffer, msgid) WHERE msgid IS NOT NULL;
         CREATE VIRTUAL TABLE IF NOT EXISTS lines_fts USING fts5 (
             text, content = 'lines', content_rowid = 'id',
             tokenize = 'unicode61 remove_diacritics 2'
         );
         CREATE TRIGGER IF NOT EXISTS lines_added AFTER INSERT ON lines BEGIN
             INSERT INTO lines_fts (rowid, text) VALUES (new.id, new.text);
         END;
         PRAGMA user_version = 1;",
    )
}

fn run(conn: &Connection, jobs: &Receiver<Job>, answers: &UnboundedSender<Answer>) {
    let mut next = None;
    loop {
        let Some(job) = next.take().or_else(|| jobs.recv().ok()) else {
            return;
        };
        match job {
            Job::Search(id, query) => {
                let _ = answers.send((id, search(conn, &query)));
            }
            Job::Add(entry) => {
                // Write what is waiting in one transaction: far faster than a
                // commit per line on a busy network.
                let mut batch = vec![entry];
                while batch.len() < BATCH {
                    match jobs.try_recv() {
                        Ok(Job::Add(entry)) => batch.push(entry),
                        Ok(other) => {
                            next = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if let Err(error) = add(conn, &batch) {
                    tracing::warn!(%error, "cannot index lines");
                }
            }
        }
    }
}

fn kind_name(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Action => "action",
        LineKind::Notice => "notice",
        _ => "message",
    }
}

fn add(conn: &Connection, batch: &[Entry]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR IGNORE INTO lines (network, buffer, time, kind, sender, text, msgid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for e in batch {
            insert.execute(params![
                e.network,
                e.buffer,
                e.time.as_millisecond(),
                kind_name(e.kind),
                e.sender,
                e.text,
                e.msgid,
            ])?;
        }
    }
    tx.commit()
}

/// Turns typed words into an FTS5 query: each word quoted (so nothing in it
/// is query syntax) and matched as a prefix, all of them required.
pub fn match_expression(words: &str) -> Option<String> {
    let terms: Vec<String> = words
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn search(conn: &Connection, query: &Query) -> Result<Vec<Hit>, String> {
    let expression =
        match_expression(&query.words).ok_or_else(|| "nothing to search for".to_owned())?;
    let limit = i64::try_from(query.limit.clamp(1, MAX_RESULTS)).unwrap_or(50);
    let mut stmt = conn
        .prepare_cached(
            "SELECT l.network, l.buffer, l.time, l.sender,
                    highlight(lines_fts, 0, char(2), char(2))
             FROM lines_fts JOIN lines l ON l.id = lines_fts.rowid
             WHERE lines_fts MATCH ?1
               AND (?2 IS NULL OR l.network = ?2)
               AND (?3 IS NULL OR l.buffer = ?3)
               AND (?4 IS NULL OR l.sender = ?4)
             ORDER BY l.time DESC, l.id DESC
             LIMIT ?5",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![expression, query.network, query.buffer, query.from, limit],
            |row| {
                let millis: i64 = row.get(2)?;
                Ok(Hit {
                    network: row.get(0)?,
                    buffer: row.get(1)?,
                    time: Timestamp::from_millisecond(millis).unwrap_or_default(),
                    sender: row.get(3)?,
                    text: row.get(4)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    let mut hits = rows
        .collect::<rusqlite::Result<Vec<Hit>>>()
        .map_err(|e| e.to_string())?;
    hits.reverse();
    Ok(hits)
}

/// How many lines the index holds (for tests and diagnostics).
pub fn count(path: &Path) -> Result<u64, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.query_row("SELECT count(*) FROM lines", [], |row| row.get::<_, i64>(0))
        .optional()
        .map(|n| n.and_then(|n| u64::try_from(n).ok()).unwrap_or(0))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(buffer: &str, minute: i64, sender: &str, text: &str, msgid: Option<&str>) -> Entry {
        Entry {
            network: "libera".into(),
            buffer: buffer.into(),
            time: Timestamp::from_second(1_800_000_000 + minute * 60).unwrap(),
            kind: LineKind::Message,
            sender: Some(sender.into()),
            text: text.into(),
            msgid: msgid.map(str::to_owned),
        }
    }

    #[test]
    fn quotes_words_so_nothing_is_syntax() {
        assert_eq!(
            match_expression("rust \"OR\" NEAR(x"),
            Some("\"rust\"* \"OR\"* \"NEAR(x\"*".into())
        );
        assert_eq!(match_expression("  \"\" "), None);
    }

    #[test]
    fn finds_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("i.sqlite")).unwrap();
        setup(&conn).unwrap();
        add(
            &conn,
            &[
                entry(
                    "#rust",
                    1,
                    "alice",
                    "the borrow checker is great",
                    Some("a"),
                ),
                entry("#rust", 2, "bob", "Borrowing again? Café time", None),
                entry("#irsty", 3, "alice", "borrowed a terminal", None),
                entry("#rust", 4, "carol", "nothing to see", None),
                // The same message from history again: indexed once.
                entry(
                    "#rust",
                    1,
                    "alice",
                    "the borrow checker is great",
                    Some("a"),
                ),
            ],
        )
        .unwrap();
        let find = |q: Query| {
            let limit = if q.limit == 0 { 50 } else { q.limit };
            search(&conn, &Query { limit, ..q }).unwrap()
        };
        let all = find(Query {
            words: "borrow".into(),
            ..Query::default()
        });
        let texts: Vec<&str> = all.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "the \x02borrow\x02 checker is great",
                "\x02Borrowing\x02 again? Café time",
                "\x02borrowed\x02 a terminal",
            ],
            "prefix matches, oldest first, matches in bold, deduplicated"
        );
        let here = find(Query {
            words: "borrow".into(),
            network: Some("libera".into()),
            buffer: Some("#RUST".into()),
            ..Query::default()
        });
        assert_eq!(here.len(), 2, "per buffer, case-insensitive");
        let from = find(Query {
            words: "borrow".into(),
            from: Some("ALICE".into()),
            ..Query::default()
        });
        assert_eq!(from.len(), 2);
        let accents = find(Query {
            words: "cafe".into(),
            ..Query::default()
        });
        assert_eq!(accents.len(), 1, "diacritics ignored");
        let limited = find(Query {
            words: "borrow".into(),
            limit: 1,
            ..Query::default()
        });
        assert_eq!(limited.len(), 1);
        assert!(limited[0].text.contains("terminal"), "the newest");
        assert!(
            search(
                &conn,
                &Query {
                    words: " ".into(),
                    ..Query::default()
                }
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn runs_on_its_own_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("i.sqlite");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let index = Index::open(&path, tx).unwrap();
        for i in 0..1000 {
            assert!(index.add(entry("#c", i, "n", &format!("line {i} hello"), None)));
        }
        assert!(index.search(
            7,
            Query {
                words: "hello".into(),
                limit: 5,
                ..Query::default()
            }
        ));
        let (id, hits) = rx.recv().await.unwrap();
        assert_eq!(id, 7);
        let hits = hits.unwrap();
        assert_eq!(hits.len(), 5);
        assert!(hits[4].text.starts_with("line 999"), "{:?}", hits[4]);
        drop(index);
        assert_eq!(count(&path).unwrap(), 1000);
    }
}
