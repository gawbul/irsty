//! Untrusted text must never produce a control character for the terminal.
#![no_main]

use irsty_proto::format::{self, StyledText};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    for styled in [StyledText::parse(text), StyledText::plain(text)] {
        let plain = styled.to_plain();
        assert!(!plain.chars().any(char::is_control), "{plain:?}");
    }
    let ident = format::sanitise_identifier(text);
    assert!(!ident.chars().any(char::is_control), "{ident:?}");
    let _ = format::strip_formatting(text);
    let _ = irsty_proto::split::split_text(text, 64);
});
