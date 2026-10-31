//! A hostile server: feed arbitrary lines to the CAP, SASL and ISUPPORT state.
#![no_main]

use irsty_proto::cap::CapNegotiator;
use irsty_proto::sasl::{Authenticator, Mechanism};
use irsty_proto::{ISupport, MessageRef, Secret};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    let mut cap = CapNegotiator::new(["server-time", "message-tags", "batch"], true);
    let mut sasl = Authenticator::new(
        Mechanism::Plain {
            account: "fuzz".into(),
            password: Secret::new("secret"),
        },
        &[],
    );
    // A fixed nonce keeps runs reproducible; its base64 is "BwcHBwcHBwcHBwcHBwcHBw==".
    let mut scram = Authenticator::new(
        Mechanism::ScramSha256 {
            account: "fuzz".into(),
            password: Secret::new("secret"),
        },
        &[7; 16],
    );
    let mut isupport = ISupport::default();
    for line in data.split('\n') {
        let Ok(msg) = MessageRef::parse(line) else {
            continue;
        };
        for out in cap.handle(&msg).send {
            assert!(out.to_wire().is_ok(), "{out:?}");
        }
        for out in sasl.handle(&msg).send.into_iter().chain(scram.handle(&msg).send) {
            assert!(out.to_wire().is_ok(), "{out:?}");
        }
        if msg.command() == "005" {
            isupport.apply(msg.params());
        }
        if msg.command() == "001" {
            let _ = cap.registered();
        }
    }
    let _ = cap.finish();
});
