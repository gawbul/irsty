//! IRC and IRCv3 protocol primitives for Irsty.
//!
//! This crate performs no I/O and has no async runtime. It parses and serialises
//! messages, tracks `RPL_ISUPPORT`, applies casemapping, and drives the CAP and SASL
//! negotiation state machines, so every piece can be tested without a socket.
#![forbid(unsafe_code)]
