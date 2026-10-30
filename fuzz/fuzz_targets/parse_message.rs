//! Raw bytes from the network: decode, parse, and round-trip what parses.
#![no_main]

use irsty_proto::{Message, MessageRef, decode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let line = decode::decode_line(data);
    let Ok(msg) = MessageRef::parse(&line) else {
        return;
    };
    let _ = msg.source();
    let _ = msg.tag("msgid");
    let owned = msg.to_owned();
    // Whatever serialises must parse back to the same message.
    if let Ok(wire) = owned.to_wire() {
        assert_eq!(Message::parse(&wire).ok(), Some(owned));
    }
});
