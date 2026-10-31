//! IRCv3 batches: messages tagged with an open batch are held until it ends,
//! then handled together (so a netsplit becomes one line, not hundreds).
//!
//! A hostile server could open batches and never close them, so the number of
//! open batches, the messages each may hold and the bytes held across all of
//! them are capped. Past the caps, messages are handled at once as if
//! unbatched.

use std::collections::HashMap;

use irsty_proto::Message;

/// Most batches open at once.
pub const MAX_OPEN: usize = 32;

/// Most messages held by one batch.
pub const MAX_MESSAGES: usize = 4096;

/// Most bytes held across every open batch. A message can be about 8.7 KB
/// with tags, so the per-batch caps alone would allow over a gigabyte.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

/// Roughly what a message costs to hold.
fn size(msg: &Message) -> usize {
    msg.command.len()
        + msg.source.as_ref().map_or(0, String::len)
        + msg.params.iter().map(String::len).sum::<usize>()
        + msg
            .tags
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
}

/// A batch that has ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    /// Its type, such as `netsplit` or `labeled-response`.
    pub kind: String,
    /// The parameters after the type.
    pub params: Vec<String>,
    /// The `label` tag on its start, for `labeled-response`.
    pub label: Option<String>,
    /// The messages it held, in order.
    pub messages: Vec<Message>,
}

/// What to do with a message.
#[derive(Debug, PartialEq, Eq)]
pub enum Routed {
    /// Handle it now.
    Now(Box<Message>),
    /// It was held, or was a batch start; nothing to do yet.
    Held,
    /// A batch ended; handle its messages.
    Ended(Batch),
}

/// The batches open on one connection.
#[derive(Clone, Debug, Default)]
pub struct Batches {
    open: HashMap<String, Batch>,
    /// Bytes held across all open batches.
    held: usize,
}

impl Batches {
    /// Decides what to do with an incoming message.
    pub fn route(&mut self, msg: Message) -> Routed {
        if msg.command.eq_ignore_ascii_case("BATCH") {
            let reference = msg.params.first().cloned().unwrap_or_default();
            if let Some(id) = reference.strip_prefix('+') {
                if !id.is_empty() && self.open.len() < MAX_OPEN {
                    let label = msg.tags.get("label").map(str::to_owned);
                    let mut params = msg.params.into_iter().skip(1);
                    let kind = params.next().unwrap_or_default();
                    self.open.insert(
                        id.to_owned(),
                        Batch {
                            kind,
                            params: params.collect(),
                            label,
                            messages: Vec::new(),
                        },
                    );
                }
                return Routed::Held;
            }
            if let Some(id) = reference.strip_prefix('-') {
                let Some(batch) = self.open.remove(id) else {
                    return Routed::Held;
                };
                let freed: usize = batch.messages.iter().map(size).sum();
                self.held = self.held.saturating_sub(freed);
                return Routed::Ended(batch);
            }
            return Routed::Held;
        }
        let batch = msg.tags.get("batch").and_then(|id| self.open.get_mut(id));
        let cost = size(&msg);
        match batch {
            Some(batch) if batch.messages.len() < MAX_MESSAGES && self.held + cost <= MAX_BYTES => {
                self.held += cost;
                batch.messages.push(msg);
                Routed::Held
            }
            _ => Routed::Now(Box::new(msg)),
        }
    }

    /// Forgets every open batch, for a new connection.
    pub fn clear(&mut self) {
        self.open.clear();
        self.held = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(line: &str) -> Message {
        Message::parse(line).unwrap()
    }

    #[test]
    fn holds_messages_until_the_batch_ends() {
        let mut b = Batches::default();
        assert_eq!(
            b.route(msg("BATCH +yXNAbvnRHTRBv netsplit irc.hub other.host")),
            Routed::Held
        );
        assert_eq!(
            b.route(msg(
                "@batch=yXNAbvnRHTRBv :aji!a@a QUIT :irc.hub other.host"
            )),
            Routed::Held
        );
        assert!(matches!(
            b.route(msg(":x!y@z PRIVMSG #c :hi")),
            Routed::Now(_)
        ));
        let Routed::Ended(batch) = b.route(msg("BATCH -yXNAbvnRHTRBv")) else {
            panic!("batch did not end");
        };
        assert_eq!(batch.kind, "netsplit");
        assert_eq!(batch.params, ["irc.hub", "other.host"]);
        assert_eq!(batch.messages.len(), 1);
        // Ending it twice, or an unknown batch, does nothing.
        assert_eq!(b.route(msg("BATCH -yXNAbvnRHTRBv")), Routed::Held);
    }

    #[test]
    fn keeps_the_label_of_a_labeled_response() {
        let mut b = Batches::default();
        b.route(msg("@label=abc BATCH +r labeled-response"));
        b.route(msg("@batch=r :srv 311 me nick u h * :real"));
        let Routed::Ended(batch) = b.route(msg("BATCH -r")) else {
            panic!();
        };
        assert_eq!(batch.label.as_deref(), Some("abc"));
    }

    #[test]
    fn caps_open_batches_and_their_size() {
        let mut b = Batches::default();
        for i in 0..MAX_OPEN + 5 {
            b.route(msg(&format!("BATCH +b{i} x")));
        }
        // The batch past the cap was never opened, so its messages pass through.
        assert!(matches!(
            b.route(msg(&format!("@batch=b{} PING x", MAX_OPEN + 1))),
            Routed::Now(_)
        ));
        for _ in 0..MAX_MESSAGES {
            assert_eq!(b.route(msg("@batch=b0 PING x")), Routed::Held);
        }
        assert!(matches!(b.route(msg("@batch=b0 PING x")), Routed::Now(_)));
        // Ending a batch frees its bytes.
        let before = b.held;
        b.route(msg("BATCH -b0"));
        assert!(b.held < before);
        // Malformed references are ignored.
        assert_eq!(b.route(msg("BATCH")), Routed::Held);
        assert_eq!(b.route(msg("BATCH +")), Routed::Held);
        b.clear();
        assert!(matches!(b.route(msg("@batch=b1 PING x")), Routed::Now(_)));
    }

    /// Big messages across many batches hit the byte budget long before the
    /// per-batch message cap.
    #[test]
    fn caps_bytes_held_across_batches() {
        let mut b = Batches::default();
        let big = "x".repeat(400);
        let mut held = 0;
        'fill: for i in 0..MAX_OPEN {
            b.route(msg(&format!("BATCH +b{i} x")));
            for _ in 0..MAX_MESSAGES {
                match b.route(msg(&format!("@batch=b{i} PRIVMSG #c :{big}"))) {
                    Routed::Held => held += 1,
                    _ => break 'fill,
                }
            }
        }
        assert!(b.held <= MAX_BYTES);
        assert!(held < MAX_OPEN * MAX_MESSAGES);
        assert!(held * 400 >= MAX_BYTES * 9 / 10, "held only {held}");
    }
}
