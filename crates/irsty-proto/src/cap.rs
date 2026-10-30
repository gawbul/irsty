//! IRCv3 capability negotiation (`CAP LS 302`) as a sans-IO state machine.
//!
//! The caller sends [`CapNegotiator::start`] along with `NICK` and `USER`, feeds
//! every `CAP` message to [`CapNegotiator::handle`], and sends what it returns. If
//! SASL is wanted and acknowledged, negotiation pauses with [`CapEvent::SaslReady`]
//! until [`CapNegotiator::finish`] is called after authentication.

use std::collections::{BTreeMap, BTreeSet};

use crate::MAX_BODY_LEN;
use crate::message::{Message, MessageRef};

/// Something the caller should know about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapEvent {
    /// Capabilities were enabled.
    Enabled(Vec<String>),
    /// Capabilities were disabled (`ACK -cap`) or withdrawn (`DEL`).
    Disabled(Vec<String>),
    /// The server refused a request.
    Rejected(Vec<String>),
    /// `sasl` was acknowledged; authenticate, then call
    /// [`CapNegotiator::finish`].
    SaslReady,
    /// SASL was wanted but the server does not offer it, or refused it.
    SaslUnavailable,
    /// `CAP END` has been sent (or the server does not support CAP) and
    /// registration can complete.
    Finished,
}

/// The result of handling a message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapStep {
    /// Messages to send, in order.
    pub send: Vec<Message>,
    /// Events, in order.
    pub events: Vec<CapEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// `CAP LS` sent, waiting for the (possibly multi-line) reply.
    Listing,
    /// `CAP REQ` batches outstanding.
    Requesting,
    /// Waiting for the caller to finish SASL.
    Sasl,
    /// Negotiation over; only `NEW`, `DEL` and their follow-ups are handled.
    Done,
}

/// Negotiates capabilities during and after registration.
#[derive(Clone, Debug)]
pub struct CapNegotiator {
    wanted: BTreeSet<String>,
    want_sasl: bool,
    available: BTreeMap<String, String>,
    enabled: BTreeSet<String>,
    outstanding: usize,
    phase: Phase,
}

impl CapNegotiator {
    /// Creates a negotiator requesting `wanted` caps when offered. If `want_sasl`
    /// is set, `sasl` is requested too and `CAP END` waits for authentication.
    pub fn new<I, S>(wanted: I, want_sasl: bool) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut wanted: BTreeSet<String> = wanted.into_iter().map(Into::into).collect();
        if want_sasl {
            wanted.insert("sasl".into());
        }
        Self {
            wanted,
            want_sasl,
            available: BTreeMap::new(),
            enabled: BTreeSet::new(),
            outstanding: 0,
            phase: Phase::Listing,
        }
    }

    /// The first message to send: `CAP LS 302`.
    pub fn start(&self) -> Message {
        Message::new("CAP").param("LS").param("302")
    }

    /// Reports whether `cap` is enabled.
    pub fn is_enabled(&self, cap: &str) -> bool {
        self.enabled.contains(cap)
    }

    /// The enabled capabilities.
    pub fn enabled(&self) -> impl Iterator<Item = &str> {
        self.enabled.iter().map(String::as_str)
    }

    /// The value the server advertised for `cap`, such as `PLAIN,EXTERNAL` for
    /// `sasl`. Returns `Some("")` for caps without a value.
    pub fn value(&self, cap: &str) -> Option<&str> {
        self.available.get(cap).map(String::as_str)
    }

    /// Reports whether negotiation has finished.
    pub fn is_finished(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Ends negotiation after SASL (successful or not). Returns `CAP END` if it
    /// still needs sending.
    pub fn finish(&mut self) -> CapStep {
        let mut step = CapStep::default();
        if self.phase != Phase::Done {
            self.end(&mut step);
        }
        step
    }

    /// Call on `RPL_WELCOME` (001), or on `ERR_UNKNOWNCOMMAND` for `CAP`, so a
    /// server without CAP support does not leave negotiation hanging.
    pub fn registered(&mut self) -> CapStep {
        let mut step = CapStep::default();
        if self.phase != Phase::Done {
            self.phase = Phase::Done;
            if self.want_sasl && !self.enabled.contains("sasl") {
                step.events.push(CapEvent::SaslUnavailable);
            }
            step.events.push(CapEvent::Finished);
        }
        step
    }

    /// Handles a `CAP` message. Other commands are ignored.
    pub fn handle(&mut self, msg: &MessageRef<'_>) -> CapStep {
        let mut step = CapStep::default();
        if !msg.command().eq_ignore_ascii_case("CAP") {
            return step;
        }
        // CAP <target> <subcommand> [*] :<caps>
        let Some(sub) = msg.param(1) else {
            return step;
        };
        let list = if msg.params().len() >= 3 {
            msg.params().last().copied().unwrap_or("")
        } else {
            ""
        };
        let more = msg.params().len() >= 4 && msg.param(2) == Some("*");
        match sub.to_ascii_uppercase().as_str() {
            "LS" => self.on_ls(list, more, &mut step),
            "NEW" => self.on_new(list, &mut step),
            "ACK" => self.on_ack(list, &mut step),
            "NAK" => self.on_nak(list, &mut step),
            "DEL" => self.on_del(list, &mut step),
            _ => {}
        }
        step
    }

    fn record_available(&mut self, list: &str) -> Vec<String> {
        let mut names = Vec::new();
        for item in list.split(' ').filter(|s| !s.is_empty()) {
            let (name, value) = item.split_once('=').unwrap_or((item, ""));
            self.available.insert(name.to_owned(), value.to_owned());
            names.push(name.to_owned());
        }
        names
    }

    fn on_ls(&mut self, list: &str, more: bool, step: &mut CapStep) {
        self.record_available(list);
        if more || self.phase != Phase::Listing {
            return;
        }
        let wanted: Vec<String> = self
            .wanted
            .iter()
            .filter(|cap| self.available.contains_key(*cap))
            .cloned()
            .collect();
        if wanted.is_empty() {
            self.end(step);
        } else {
            self.phase = Phase::Requesting;
            self.request(&wanted, step);
        }
    }

    fn on_new(&mut self, list: &str, step: &mut CapStep) {
        let names = self.record_available(list);
        let wanted: Vec<String> = names
            .into_iter()
            .filter(|n| self.wanted.contains(n) && !self.enabled.contains(n))
            .collect();
        self.request(&wanted, step);
    }

    fn on_ack(&mut self, list: &str, step: &mut CapStep) {
        let (mut on, mut off) = (Vec::new(), Vec::new());
        for item in list.split(' ').filter(|s| !s.is_empty()) {
            if let Some(name) = item.strip_prefix('-') {
                self.enabled.remove(name);
                off.push(name.to_owned());
            } else {
                self.enabled.insert(item.to_owned());
                on.push(item.to_owned());
            }
        }
        if !on.is_empty() {
            step.events.push(CapEvent::Enabled(on));
        }
        if !off.is_empty() {
            step.events.push(CapEvent::Disabled(off));
        }
        self.answered(step);
    }

    fn on_nak(&mut self, list: &str, step: &mut CapStep) {
        let caps: Vec<String> = list
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        // A REQ is all or nothing, so one unsupported cap sinks the batch. Retry
        // each cap on its own so the others still get enabled.
        if caps.len() > 1 {
            self.outstanding = self.outstanding.saturating_sub(1);
            for cap in &caps {
                self.request(std::slice::from_ref(cap), step);
            }
            return;
        }
        step.events.push(CapEvent::Rejected(caps));
        self.answered(step);
    }

    fn on_del(&mut self, list: &str, step: &mut CapStep) {
        let mut removed = Vec::new();
        for name in list.split(' ').filter(|s| !s.is_empty()) {
            self.available.remove(name);
            if self.enabled.remove(name) {
                removed.push(name.to_owned());
            }
        }
        if !removed.is_empty() {
            step.events.push(CapEvent::Disabled(removed));
        }
    }

    /// Sends `CAP REQ` in as few lines as fit.
    fn request(&mut self, caps: &[String], step: &mut CapStep) {
        // "CAP REQ :" plus the list must fit in the body limit.
        let budget = MAX_BODY_LEN - "CAP REQ :".len();
        let mut line = String::new();
        for cap in caps {
            if !line.is_empty() && line.len() + 1 + cap.len() > budget {
                self.send_req(std::mem::take(&mut line), step);
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(cap);
        }
        if !line.is_empty() {
            self.send_req(line, step);
        }
    }

    fn send_req(&mut self, caps: String, step: &mut CapStep) {
        self.outstanding += 1;
        step.send.push(Message::new("CAP").param("REQ").param(caps));
    }

    /// Records a reply to a `REQ` and moves on once all are answered.
    fn answered(&mut self, step: &mut CapStep) {
        self.outstanding = self.outstanding.saturating_sub(1);
        if self.outstanding > 0 || self.phase != Phase::Requesting {
            return;
        }
        if self.want_sasl && self.enabled.contains("sasl") {
            self.phase = Phase::Sasl;
            step.events.push(CapEvent::SaslReady);
        } else {
            self.end(step);
        }
    }

    fn end(&mut self, step: &mut CapStep) {
        if self.want_sasl && !self.enabled.contains("sasl") {
            step.events.push(CapEvent::SaslUnavailable);
        }
        self.phase = Phase::Done;
        step.send.push(Message::new("CAP").param("END"));
        step.events.push(CapEvent::Finished);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(n: &mut CapNegotiator, line: &str) -> CapStep {
        n.handle(&MessageRef::parse(line).unwrap())
    }

    fn wire(step: &CapStep) -> Vec<String> {
        step.send.iter().map(|m| m.to_wire().unwrap()).collect()
    }

    #[test]
    fn multiline_ls_then_req_ack_end() {
        let mut n = CapNegotiator::new(["server-time", "message-tags", "away-notify"], false);
        assert_eq!(n.start().to_wire().unwrap(), "CAP LS 302");
        let step = feed(&mut n, ":srv CAP * LS * :server-time sasl=PLAIN,EXTERNAL");
        assert!(step.send.is_empty());
        let step = feed(&mut n, ":srv CAP * LS :message-tags chghost");
        assert_eq!(wire(&step), ["CAP REQ :message-tags server-time"]);
        assert_eq!(n.value("sasl"), Some("PLAIN,EXTERNAL"));
        assert_eq!(n.value("chghost"), Some(""));
        let step = feed(&mut n, ":srv CAP * ACK :message-tags server-time");
        assert_eq!(wire(&step), ["CAP END"]);
        assert_eq!(
            step.events,
            [
                CapEvent::Enabled(vec!["message-tags".into(), "server-time".into()]),
                CapEvent::Finished
            ]
        );
        assert!(n.is_finished());
        assert!(n.is_enabled("server-time"));
        assert_eq!(n.enabled().count(), 2);
    }

    #[test]
    fn nothing_wanted_ends_immediately() {
        let mut n = CapNegotiator::new(["echo-message"], false);
        let step = feed(&mut n, "CAP * LS :chghost");
        assert_eq!(wire(&step), ["CAP END"]);
    }

    #[test]
    fn sasl_pauses_until_finish() {
        let mut n = CapNegotiator::new(["server-time"], true);
        let step = feed(&mut n, "CAP * LS :sasl server-time");
        assert_eq!(wire(&step), ["CAP REQ :sasl server-time"]);
        let step = feed(&mut n, "CAP * ACK :sasl server-time");
        assert!(step.send.is_empty());
        assert_eq!(step.events.last(), Some(&CapEvent::SaslReady));
        assert!(!n.is_finished());
        assert_eq!(wire(&n.finish()), ["CAP END"]);
        assert!(n.finish().send.is_empty());
    }

    #[test]
    fn sasl_missing_is_reported() {
        let mut n = CapNegotiator::new(Vec::<String>::new(), true);
        let step = feed(&mut n, "CAP * LS :server-time");
        assert_eq!(step.events, [CapEvent::SaslUnavailable, CapEvent::Finished]);
    }

    #[test]
    fn nak_of_batch_retries_individually() {
        let mut n = CapNegotiator::new(["a", "b"], false);
        feed(&mut n, "CAP * LS :a b");
        let step = feed(&mut n, "CAP * NAK :a b");
        assert_eq!(wire(&step), ["CAP REQ a", "CAP REQ b"]);
        let step = feed(&mut n, "CAP * ACK :a");
        assert!(step.send.is_empty());
        let step = feed(&mut n, "CAP * NAK :b");
        assert_eq!(
            step.events,
            [CapEvent::Rejected(vec!["b".into()]), CapEvent::Finished]
        );
        assert_eq!(wire(&step), ["CAP END"]);
        assert!(n.is_enabled("a") && !n.is_enabled("b"));
    }

    #[test]
    fn new_and_del_after_registration() {
        let mut n = CapNegotiator::new(["away-notify", "cap-notify"], false);
        feed(&mut n, "CAP * LS :cap-notify");
        feed(&mut n, "CAP * ACK :cap-notify");
        let step = feed(&mut n, "CAP nick NEW :away-notify other");
        assert_eq!(wire(&step), ["CAP REQ away-notify"]);
        let step = feed(&mut n, "CAP nick ACK :away-notify");
        assert!(step.send.is_empty(), "no second CAP END");
        let step = feed(&mut n, "CAP nick DEL :away-notify other");
        assert_eq!(
            step.events,
            [CapEvent::Disabled(vec!["away-notify".into()])]
        );
        assert_eq!(n.value("away-notify"), None);
        let step = feed(&mut n, "CAP nick ACK :-cap-notify");
        assert_eq!(step.events, [CapEvent::Disabled(vec!["cap-notify".into()])]);
    }

    #[test]
    fn long_requests_are_split() {
        let caps: Vec<String> = (0..60)
            .map(|i| format!("vendor.example/capability-{i:02}"))
            .collect();
        let mut n = CapNegotiator::new(caps.clone(), false);
        let step = feed(&mut n, &format!("CAP * LS :{}", caps.join(" ")));
        assert!(step.send.len() > 1);
        for msg in &step.send {
            assert!(msg.to_wire().unwrap().len() <= MAX_BODY_LEN);
        }
    }

    #[test]
    fn registered_without_cap_support() {
        let mut n = CapNegotiator::new(["x"], true);
        let step = n.registered();
        assert_eq!(step.events, [CapEvent::SaslUnavailable, CapEvent::Finished]);
        assert!(step.send.is_empty());
        assert!(n.registered().events.is_empty());
    }

    #[test]
    fn ignores_other_and_malformed_messages() {
        let mut n = CapNegotiator::new(["x"], false);
        assert_eq!(feed(&mut n, "PING :x"), CapStep::default());
        assert_eq!(feed(&mut n, "CAP"), CapStep::default());
        assert_eq!(feed(&mut n, "CAP * LIST :x"), CapStep::default());
    }
}
