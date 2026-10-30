//! SASL authentication over `AUTHENTICATE` as a sans-IO state machine.
//!
//! Start with [`Authenticator::start`] once `sasl` is acknowledged, then feed every
//! `AUTHENTICATE` message and SASL numeric (900 to 908) to
//! [`Authenticator::handle`]. When it reports an outcome, call
//! [`crate::cap::CapNegotiator::finish`].

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::message::{Message, MessageRef};
use crate::secret::Secret;

/// Maximum payload per `AUTHENTICATE` line.
const CHUNK: usize = 400;

/// Upper bound on a server challenge, to cap memory use from a hostile server.
const MAX_CHALLENGE: usize = 64 * 1024;

/// A SASL mechanism with its credentials.
#[derive(Clone, Debug)]
pub enum Mechanism {
    /// `PLAIN`: account name and password.
    Plain {
        /// Account (authentication identity).
        account: String,
        /// Password.
        password: Secret,
    },
    /// `EXTERNAL`: authenticate with the TLS client certificate (CertFP).
    External,
}

impl Mechanism {
    /// The mechanism name sent in `AUTHENTICATE`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Plain { .. } => "PLAIN",
            Self::External => "EXTERNAL",
        }
    }

    /// Produces the client's response to a (decoded) server challenge.
    fn respond(&self, _challenge: &[u8]) -> Vec<u8> {
        match self {
            // authzid NUL authcid NUL password, with an empty authzid.
            Self::Plain { account, password } => {
                let mut out = Vec::with_capacity(account.len() * 2 + password.expose().len() + 2);
                out.extend_from_slice(account.as_bytes());
                out.push(0);
                out.extend_from_slice(account.as_bytes());
                out.push(0);
                out.extend_from_slice(password.expose().as_bytes());
                out
            }
            Self::External => Vec::new(),
        }
    }
}

/// How authentication ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Authenticated (903).
    Success,
    /// Failed: wrong credentials, aborted, or an unsupported mechanism. Carries
    /// the server's text.
    Failure(String),
    /// The server says we are already authenticated (907).
    AlreadyAuthenticated,
}

/// The result of handling a message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaslStep {
    /// Messages to send, in order.
    pub send: Vec<Message>,
    /// The account we are logged in as, from 900.
    pub logged_in_as: Option<String>,
    /// Mechanisms the server supports, from 908.
    pub server_mechanisms: Option<Vec<String>>,
    /// Set once authentication has ended.
    pub outcome: Option<Outcome>,
}

/// Runs one SASL exchange.
#[derive(Debug)]
pub struct Authenticator {
    mechanism: Mechanism,
    challenge: Vec<u8>,
    done: bool,
}

impl Authenticator {
    /// Prepares to authenticate with `mechanism`.
    pub fn new(mechanism: Mechanism) -> Self {
        Self {
            mechanism,
            challenge: Vec::new(),
            done: false,
        }
    }

    /// The first message to send: `AUTHENTICATE <mechanism>`.
    pub fn start(&self) -> Message {
        Message::new("AUTHENTICATE").param(self.mechanism.name())
    }

    /// Message to abort an exchange in progress.
    pub fn abort() -> Message {
        Message::new("AUTHENTICATE").param("*")
    }

    /// Handles `AUTHENTICATE` and the SASL numerics. Other messages are ignored.
    pub fn handle(&mut self, msg: &MessageRef<'_>) -> SaslStep {
        let mut step = SaslStep::default();
        if self.done {
            return step;
        }
        let trailing = msg.params().last().copied().unwrap_or("");
        match msg.command() {
            c if c.eq_ignore_ascii_case("AUTHENTICATE") => {
                self.on_challenge(msg.param(0).unwrap_or(""), &mut step);
            }
            "900" => step.logged_in_as = msg.param(2).map(str::to_owned),
            "903" => self.finish(Outcome::Success, &mut step),
            "907" => self.finish(Outcome::AlreadyAuthenticated, &mut step),
            "902" | "904" | "905" | "906" => {
                self.finish(Outcome::Failure(trailing.to_owned()), &mut step);
            }
            "908" => {
                step.server_mechanisms = msg.param(1).map(|m| {
                    m.split(',')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                });
            }
            _ => {}
        }
        step
    }

    fn finish(&mut self, outcome: Outcome, step: &mut SaslStep) {
        self.done = true;
        self.challenge.clear();
        step.outcome = Some(outcome);
    }

    fn on_challenge(&mut self, chunk: &str, step: &mut SaslStep) {
        // A 400-byte chunk means more follows; "+" is an empty chunk.
        if chunk != "+" {
            let Ok(decoded) = BASE64.decode(chunk) else {
                step.send.push(Self::abort());
                return;
            };
            if self.challenge.len() + decoded.len() > MAX_CHALLENGE {
                step.send.push(Self::abort());
                return;
            }
            self.challenge.extend_from_slice(&decoded);
            if chunk.len() == CHUNK {
                return;
            }
        }
        let response = self.mechanism.respond(&std::mem::take(&mut self.challenge));
        step.send.extend(encode_response(&response));
    }
}

/// Encodes a response as `AUTHENTICATE` lines of at most 400 bytes. An empty
/// response, or one that is an exact multiple of 400 bytes once encoded, ends with
/// `AUTHENTICATE +`.
fn encode_response(response: &[u8]) -> Vec<Message> {
    let encoded = BASE64.encode(response);
    let mut lines: Vec<Message> = encoded
        .as_bytes()
        .chunks(CHUNK)
        .map(|c| Message::new("AUTHENTICATE").param(String::from_utf8_lossy(c)))
        .collect();
    if encoded.len().is_multiple_of(CHUNK) {
        lines.push(Message::new("AUTHENTICATE").param("+"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(a: &mut Authenticator, line: &str) -> SaslStep {
        a.handle(&MessageRef::parse(line).unwrap())
    }

    fn wire(step: &SaslStep) -> Vec<String> {
        step.send.iter().map(|m| m.to_wire().unwrap()).collect()
    }

    fn plain(account: &str, password: &str) -> Authenticator {
        Authenticator::new(Mechanism::Plain {
            account: account.into(),
            password: Secret::new(password),
        })
    }

    #[test]
    fn plain_exchange() {
        let mut a = plain("jilles", "sesame");
        assert_eq!(a.start().to_wire().unwrap(), "AUTHENTICATE PLAIN");
        let step = feed(&mut a, "AUTHENTICATE +");
        // Example from the IRCv3 SASL specification.
        assert_eq!(wire(&step), ["AUTHENTICATE amlsbGVzAGppbGxlcwBzZXNhbWU="]);
        let step = feed(
            &mut a,
            ":srv 900 nick nick!u@h jilles :You are now logged in as jilles",
        );
        assert_eq!(step.logged_in_as.as_deref(), Some("jilles"));
        let step = feed(&mut a, ":srv 903 nick :SASL authentication successful");
        assert_eq!(step.outcome, Some(Outcome::Success));
        assert_eq!(feed(&mut a, "AUTHENTICATE +"), SaslStep::default());
    }

    #[test]
    fn external_sends_empty_response() {
        let mut a = Authenticator::new(Mechanism::External);
        assert_eq!(a.start().to_wire().unwrap(), "AUTHENTICATE EXTERNAL");
        assert_eq!(wire(&feed(&mut a, "AUTHENTICATE +")), ["AUTHENTICATE +"]);
    }

    #[test]
    fn long_responses_are_chunked() {
        // 300 bytes of password encodes to well over 400 characters.
        let mut a = plain("acct", &"p".repeat(300));
        let lines = wire(&feed(&mut a, "AUTHENTICATE +"));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), "AUTHENTICATE ".len() + CHUNK);

        // A response that encodes to exactly 400 characters needs a "+" after it.
        let exact = encode_response(&[0u8; 300]);
        assert_eq!(exact.len(), 2);
        assert_eq!(exact[1].to_wire().unwrap(), "AUTHENTICATE +");
    }

    #[test]
    fn multi_chunk_challenges_are_buffered() {
        let mut a = Authenticator::new(Mechanism::External);
        let first = "A".repeat(CHUNK);
        assert!(
            feed(&mut a, &format!("AUTHENTICATE {first}"))
                .send
                .is_empty()
        );
        assert_eq!(wire(&feed(&mut a, "AUTHENTICATE QQ==")), ["AUTHENTICATE +"]);
    }

    #[test]
    fn bad_challenges_abort() {
        let mut a = plain("a", "b");
        assert_eq!(wire(&feed(&mut a, "AUTHENTICATE !!!")), ["AUTHENTICATE *"]);
        let chunk = "A".repeat(CHUNK);
        for _ in 0..(MAX_CHALLENGE / 300) {
            feed(&mut a, &format!("AUTHENTICATE {chunk}"));
        }
        assert_eq!(
            wire(&feed(&mut a, &format!("AUTHENTICATE {chunk}"))),
            ["AUTHENTICATE *"]
        );
    }

    #[test]
    fn failures_and_mechanism_lists() {
        let mut a = plain("a", "b");
        let step = feed(
            &mut a,
            ":srv 908 nick EXTERNAL,SCRAM-SHA-256 :are available SASL mechanisms",
        );
        assert_eq!(
            step.server_mechanisms,
            Some(vec!["EXTERNAL".into(), "SCRAM-SHA-256".into()])
        );
        let step = feed(&mut a, ":srv 904 nick :SASL authentication failed");
        assert_eq!(
            step.outcome,
            Some(Outcome::Failure("SASL authentication failed".into()))
        );

        let mut b = plain("a", "b");
        assert_eq!(
            feed(&mut b, ":srv 907 nick :already").outcome,
            Some(Outcome::AlreadyAuthenticated)
        );
        assert_eq!(feed(&mut b, "PING x"), SaslStep::default());
    }

    #[test]
    fn debug_never_shows_password() {
        let a = plain("acct", "hunter2");
        assert!(!format!("{a:?}").contains("hunter2"));
    }
}
