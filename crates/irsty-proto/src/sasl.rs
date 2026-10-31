//! SASL authentication over `AUTHENTICATE` as a sans-IO state machine.
//!
//! Start with [`Authenticator::start`] once `sasl` is acknowledged, then feed every
//! `AUTHENTICATE` message and SASL numeric (900 to 908) to
//! [`Authenticator::handle`]. When it reports an outcome, call
//! [`crate::cap::CapNegotiator::finish`].

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

use crate::message::{Message, MessageRef};
use crate::secret::Secret;

/// Maximum payload per `AUTHENTICATE` line.
const CHUNK: usize = 400;

/// Upper bound on a server challenge, to cap memory use from a hostile server.
const MAX_CHALLENGE: usize = 64 * 1024;

/// SCRAM iteration counts we accept. RFC 7677 asks for at least 4096; the cap
/// stops a hostile server making us spin on PBKDF2.
const SCRAM_ITERATIONS: std::ops::RangeInclusive<u32> = 4096..=600_000;

/// Shortest client nonce we accept for SCRAM, in bytes of randomness.
pub const MIN_NONCE: usize = 16;

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
    /// `SCRAM-SHA-256` (RFC 7677): proves the password without sending it, and
    /// checks that the server knows it too.
    ScramSha256 {
        /// Account (authentication identity).
        account: String,
        /// Password.
        password: Secret,
    },
}

impl Mechanism {
    /// The mechanism name sent in `AUTHENTICATE`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Plain { .. } => "PLAIN",
            Self::External => "EXTERNAL",
            Self::ScramSha256 { .. } => "SCRAM-SHA-256",
        }
    }
}

/// Where a SCRAM exchange has got to. Keys are kept out of `Debug`.
#[derive(Default)]
enum Scram {
    /// Nothing sent yet.
    #[default]
    Start,
    /// Sent client-first; waiting for server-first.
    SentFirst {
        /// `n=<user>,r=<client nonce>`.
        bare: String,
    },
    /// Sent client-final; waiting for the server's signature.
    SentFinal {
        /// What the server must prove.
        server_signature: Vec<u8>,
    },
    /// The server proved it knows the password.
    Verified,
}

impl std::fmt::Debug for Scram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Start => "Start",
            Self::SentFirst { .. } => "SentFirst",
            Self::SentFinal { .. } => "SentFinal",
            Self::Verified => "Verified",
        })
    }
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    // HMAC accepts keys of any length, so this never fails.
    HmacSha256::new_from_slice(key).map_or_else(
        |_| Vec::new(),
        |mut mac| {
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        },
    )
}

/// Applies `SASLprep` to a user name and escapes it for SCRAM (`=` and `,` are special).
fn scram_name(name: &str) -> Result<String, String> {
    let prepped = stringprep::saslprep(name)
        .map_err(|_| "the account name contains characters SASLprep forbids".to_owned())?;
    Ok(prepped.replace('=', "=3D").replace(',', "=2C"))
}

/// Parses `a=1,b=2` attributes, rejecting unknown mandatory extensions (`m=`).
fn scram_attributes(message: &str) -> Result<Vec<(char, &str)>, String> {
    message
        .split(',')
        .map(|part| {
            let mut chars = part.chars();
            match (chars.next(), chars.next()) {
                (Some('m'), Some('=')) => {
                    Err("the server requires an unsupported SCRAM extension".to_owned())
                }
                (Some(key), Some('=')) if key.is_ascii_alphabetic() => Ok((key, chars.as_str())),
                _ => Err("malformed SCRAM message from the server".to_owned()),
            }
        })
        .collect()
}

fn attribute<'a>(attrs: &[(char, &'a str)], key: char) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
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
    /// Client nonce for SCRAM, already printable; `None` if too little
    /// randomness was given.
    nonce: Option<String>,
    scram: Scram,
    /// Why we aborted, which explains the failure better than the server's 904.
    error: Option<String>,
}

impl Authenticator {
    /// Prepares to authenticate with `mechanism`. `random` must be fresh random
    /// bytes, at least [`MIN_NONCE`] of them; SCRAM uses them as its nonce and
    /// fails without them. Other mechanisms ignore them.
    pub fn new(mechanism: Mechanism, random: &[u8]) -> Self {
        Self {
            mechanism,
            challenge: Vec::new(),
            done: false,
            nonce: (random.len() >= MIN_NONCE).then(|| BASE64.encode(random)),
            scram: Scram::Start,
            error: None,
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
            "903" => {
                // SCRAM succeeds only once the server has proved itself; a 903
                // before that would let an impostor skip the check.
                let outcome = if matches!(self.mechanism, Mechanism::ScramSha256 { .. })
                    && !matches!(self.scram, Scram::Verified)
                {
                    Outcome::Failure(
                        "the server claimed success without proving it knows the password".into(),
                    )
                } else {
                    Outcome::Success
                };
                self.finish(outcome, &mut step);
            }
            "907" => self.finish(Outcome::AlreadyAuthenticated, &mut step),
            "902" | "904" | "905" | "906" => {
                let reason = self.error.take().unwrap_or_else(|| trailing.to_owned());
                self.finish(Outcome::Failure(reason), &mut step);
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
        let challenge = std::mem::take(&mut self.challenge);
        match self.respond(&challenge) {
            Ok(response) => step.send.extend(encode_response(&response)),
            Err(reason) => {
                self.error = Some(reason);
                step.send.push(Self::abort());
            }
        }
    }

    /// Produces the client's response to a (decoded) server challenge.
    fn respond(&mut self, challenge: &[u8]) -> Result<Vec<u8>, String> {
        match &self.mechanism {
            // authzid NUL authcid NUL password, with an empty authzid.
            Mechanism::Plain { account, password } => {
                let mut out = Vec::with_capacity(account.len() * 2 + password.expose().len() + 2);
                out.extend_from_slice(account.as_bytes());
                out.push(0);
                out.extend_from_slice(account.as_bytes());
                out.push(0);
                out.extend_from_slice(password.expose().as_bytes());
                Ok(out)
            }
            Mechanism::External => Ok(Vec::new()),
            Mechanism::ScramSha256 { account, password } => {
                let (account, password) = (account.clone(), password.clone());
                self.scram_step(&account, &password, challenge)
            }
        }
    }

    fn scram_step(
        &mut self,
        account: &str,
        password: &Secret,
        challenge: &[u8],
    ) -> Result<Vec<u8>, String> {
        let challenge = std::str::from_utf8(challenge)
            .map_err(|_| "malformed SCRAM message from the server".to_owned())?;
        match std::mem::take(&mut self.scram) {
            Scram::Start => {
                let Some(nonce) = &self.nonce else {
                    return Err("not enough randomness for a SCRAM nonce".into());
                };
                let bare = format!("n={},r={nonce}", scram_name(account)?);
                // "n,," : no channel binding, no authorisation identity.
                let first = format!("n,,{bare}");
                self.scram = Scram::SentFirst { bare };
                Ok(first.into_bytes())
            }
            Scram::SentFirst { bare } => {
                let attrs = scram_attributes(challenge)?;
                let malformed = || "malformed SCRAM message from the server".to_owned();
                let nonce = attribute(&attrs, 'r').ok_or_else(malformed)?;
                let salt = attribute(&attrs, 's')
                    .and_then(|s| BASE64.decode(s).ok())
                    .ok_or_else(malformed)?;
                let iterations: u32 = attribute(&attrs, 'i')
                    .and_then(|i| i.parse().ok())
                    .ok_or_else(malformed)?;
                let ours = self.nonce.as_deref().unwrap_or_default();
                if ours.is_empty() || !nonce.starts_with(ours) || nonce.len() == ours.len() {
                    return Err("the server's SCRAM nonce does not extend ours".into());
                }
                if !SCRAM_ITERATIONS.contains(&iterations) {
                    return Err(format!(
                        "the server asked for {iterations} SCRAM iterations; Irsty accepts {} to {}",
                        SCRAM_ITERATIONS.start(),
                        SCRAM_ITERATIONS.end()
                    ));
                }
                let password = stringprep::saslprep(password.expose())
                    .map_err(|_| "the password contains characters SASLprep forbids".to_owned())?;
                let mut salted = [0u8; 32];
                pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations, &mut salted);
                let client_key = hmac(&salted, b"Client Key");
                let stored_key = Sha256::digest(&client_key);
                // "biws" is base64 of the "n,," header.
                let without_proof = format!("c=biws,r={nonce}");
                let auth_message = format!("{bare},{challenge},{without_proof}");
                let signature = hmac(&stored_key, auth_message.as_bytes());
                let proof: Vec<u8> = client_key
                    .iter()
                    .zip(&signature)
                    .map(|(k, s)| k ^ s)
                    .collect();
                let server_key = hmac(&salted, b"Server Key");
                self.scram = Scram::SentFinal {
                    server_signature: hmac(&server_key, auth_message.as_bytes()),
                };
                Ok(format!("{without_proof},p={}", BASE64.encode(proof)).into_bytes())
            }
            Scram::SentFinal { server_signature } => {
                let attrs = scram_attributes(challenge)?;
                if let Some(error) = attribute(&attrs, 'e') {
                    return Err(format!("the server rejected SCRAM: {error}"));
                }
                let verified = attribute(&attrs, 'v')
                    .and_then(|v| BASE64.decode(v).ok())
                    .is_some_and(|v| constant_time_eq(&v, &server_signature));
                if !verified {
                    return Err("the server could not prove it knows the password".into());
                }
                self.scram = Scram::Verified;
                Ok(Vec::new())
            }
            Scram::Verified => Err("unexpected SCRAM message after authentication".into()),
        }
    }
}

/// Compares without an early exit, so timing reveals nothing about where they differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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

    const RANDOM: [u8; MIN_NONCE] = [7; MIN_NONCE];

    fn plain(account: &str, password: &str) -> Authenticator {
        Authenticator::new(
            Mechanism::Plain {
                account: account.into(),
                password: Secret::new(password),
            },
            &RANDOM,
        )
    }

    fn scram(account: &str, password: &str, nonce: &str) -> Authenticator {
        let mut a = Authenticator::new(
            Mechanism::ScramSha256 {
                account: account.into(),
                password: Secret::new(password),
            },
            &RANDOM,
        );
        a.nonce = Some(nonce.into());
        a
    }

    /// Feeds a decoded server message as `AUTHENTICATE` and decodes the reply.
    fn exchange(a: &mut Authenticator, server: &str) -> Vec<String> {
        let payload = if server.is_empty() {
            "+".to_owned()
        } else {
            BASE64.encode(server)
        };
        wire(&feed(a, &format!("AUTHENTICATE {payload}")))
            .iter()
            .map(|line| {
                let arg = line.strip_prefix("AUTHENTICATE ").unwrap();
                if arg == "+" || arg == "*" {
                    arg.to_owned()
                } else {
                    String::from_utf8(BASE64.decode(arg).unwrap()).unwrap()
                }
            })
            .collect()
    }

    const RFC_NONCE: &str = "rOprNGfwEbeRWgbNEkqO";
    const RFC_SERVER_FIRST: &str =
        "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";

    /// The example exchange from RFC 7677 section 3.
    #[test]
    fn scram_sha256_rfc7677_vector() {
        let mut a = scram("user", "pencil", RFC_NONCE);
        assert_eq!(a.start().to_wire().unwrap(), "AUTHENTICATE SCRAM-SHA-256");
        assert_eq!(exchange(&mut a, ""), ["n,,n=user,r=rOprNGfwEbeRWgbNEkqO"]);
        assert_eq!(
            exchange(&mut a, RFC_SERVER_FIRST),
            [
                "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,\
              p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
            ]
        );
        assert_eq!(
            exchange(&mut a, "v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4="),
            ["+"]
        );
        let step = feed(&mut a, ":srv 903 nick :SASL authentication successful");
        assert_eq!(step.outcome, Some(Outcome::Success));
    }

    #[test]
    fn scram_rejects_a_server_that_cannot_prove_itself() {
        let mut a = scram("user", "pencil", RFC_NONCE);
        exchange(&mut a, "");
        exchange(&mut a, RFC_SERVER_FIRST);
        assert_eq!(exchange(&mut a, "v=AAAA"), ["*"]);
        let step = feed(&mut a, ":srv 906 nick :SASL authentication aborted");
        assert_eq!(
            step.outcome,
            Some(Outcome::Failure(
                "the server could not prove it knows the password".into()
            ))
        );

        // A 903 before the server's signature is a failure, not a success.
        let mut b = scram("user", "pencil", RFC_NONCE);
        exchange(&mut b, "");
        exchange(&mut b, RFC_SERVER_FIRST);
        let step = feed(&mut b, ":srv 903 nick :SASL authentication successful");
        assert!(matches!(step.outcome, Some(Outcome::Failure(_))));
    }

    #[test]
    fn scram_rejects_hostile_server_first_messages() {
        let cases = [
            // Nonce doesn't extend ours, or doesn't extend it at all.
            ("r=somethingelse,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096", "nonce"),
            (
                "r=rOprNGfwEbeRWgbNEkqO,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096",
                "nonce",
            ),
            // Too few or too many iterations.
            (
                "r=rOprNGfwEbeRWgbNEkqOx,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=1",
                "iterations",
            ),
            (
                "r=rOprNGfwEbeRWgbNEkqOx,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4000000000",
                "iterations",
            ),
            ("m=ext,r=rOprNGfwEbeRWgbNEkqOx,s=AA==,i=4096", "extension"),
            ("r=rOprNGfwEbeRWgbNEkqOx,s=!!!,i=4096", "malformed"),
            ("garbage", "malformed"),
            ("e=other-error", "malformed"),
        ];
        for (server_first, why) in cases {
            let mut a = scram("user", "pencil", RFC_NONCE);
            exchange(&mut a, "");
            assert_eq!(exchange(&mut a, server_first), ["*"], "{server_first}");
            let step = feed(&mut a, ":srv 906 nick :aborted");
            let Some(Outcome::Failure(reason)) = step.outcome else {
                panic!("{server_first}");
            };
            assert!(reason.contains(why), "{server_first}: {reason}");
        }
        // The server reporting an error in server-final.
        let mut a = scram("user", "pencil", RFC_NONCE);
        exchange(&mut a, "");
        exchange(&mut a, RFC_SERVER_FIRST);
        assert_eq!(exchange(&mut a, "e=invalid-proof"), ["*"]);
        let step = feed(&mut a, ":srv 904 nick :failed");
        assert_eq!(
            step.outcome,
            Some(Outcome::Failure(
                "the server rejected SCRAM: invalid-proof".into()
            ))
        );
    }

    #[test]
    fn scram_escapes_names_and_needs_a_nonce() {
        let mut a = scram("a=b,c", "pencil", RFC_NONCE);
        assert_eq!(
            exchange(&mut a, ""),
            ["n,,n=a=3Db=2Cc,r=rOprNGfwEbeRWgbNEkqO"]
        );

        let mut short = Authenticator::new(
            Mechanism::ScramSha256 {
                account: "user".into(),
                password: Secret::new("pencil"),
            },
            &[1, 2, 3],
        );
        assert_eq!(exchange(&mut short, ""), ["*"]);
        // SASLprep forbids control characters in names.
        let mut bad = scram("bad\u{7}name", "pencil", RFC_NONCE);
        assert_eq!(exchange(&mut bad, ""), ["*"]);
    }

    #[test]
    fn scram_debug_hides_keys() {
        let mut a = scram("user", "pencil", RFC_NONCE);
        exchange(&mut a, "");
        exchange(&mut a, RFC_SERVER_FIRST);
        let debug = format!("{a:?}");
        assert!(
            !debug.contains("pencil") && debug.contains("SentFinal"),
            "{debug}"
        );
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
        let mut a = Authenticator::new(Mechanism::External, &[]);
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
        let mut a = Authenticator::new(Mechanism::External, &[]);
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
