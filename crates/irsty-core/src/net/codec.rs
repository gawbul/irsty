//! Line framing for IRC over a byte stream.

use bytes::{BufMut, BytesMut};
use irsty_proto::{MAX_LINE_LEN, Message, decode};
use tokio_util::codec::{Decoder, Encoder};

/// Splits a stream into parsed [`Message`]s and writes pre-serialised lines.
///
/// Lines longer than [`MAX_LINE_LEN`] are discarded up to the next newline, and
/// lines that do not parse are skipped, so a hostile server cannot make the
/// decoder buffer without bound or stop the connection.
#[derive(Debug, Default)]
pub struct IrcCodec {
    /// Bytes already scanned for a newline.
    scanned: usize,
    /// Inside an over-long line being thrown away.
    discarding: bool,
}

impl Decoder for IrcCodec {
    type Item = Message;
    type Error = std::io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Message>, Self::Error> {
        loop {
            let newline = buf
                .get(self.scanned..)
                .and_then(|rest| rest.iter().position(|&b| b == b'\n'))
                .map(|offset| self.scanned + offset);
            let Some(newline) = newline else {
                if buf.len() > MAX_LINE_LEN {
                    tracing::warn!(bytes = buf.len(), "discarding over-long line");
                    buf.clear();
                    self.discarding = true;
                    self.scanned = 0;
                } else {
                    self.scanned = buf.len();
                }
                return Ok(None);
            };
            self.scanned = 0;
            let line = buf.split_to(newline + 1);
            if std::mem::take(&mut self.discarding) {
                continue;
            }
            let text = decode::decode_line(line.get(..newline).unwrap_or_default());
            match Message::parse(&text) {
                Ok(msg) => return Ok(Some(msg)),
                Err(irsty_proto::ParseError::Empty) => {}
                Err(error) => tracing::debug!(%error, "skipping unparsable line"),
            }
        }
    }
}

impl Encoder<String> for IrcCodec {
    type Error = std::io::Error;

    fn encode(&mut self, line: String, buf: &mut BytesMut) -> Result<(), Self::Error> {
        buf.reserve(line.len() + 2);
        buf.put_slice(line.as_bytes());
        buf.put_slice(b"\r\n");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(codec: &mut IrcCodec, buf: &mut BytesMut) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(msg) = codec.decode(buf).unwrap() {
            out.push(msg.command);
        }
        out
    }

    #[test]
    fn splits_lines_across_reads() {
        let mut codec = IrcCodec::default();
        let mut buf = BytesMut::from("PING :a\r\nPRIV");
        assert_eq!(decode_all(&mut codec, &mut buf), ["PING"]);
        buf.extend_from_slice(b"MSG #c :hi\n\r\n\nNOTICE x :y\r\n");
        assert_eq!(decode_all(&mut codec, &mut buf), ["PRIVMSG", "NOTICE"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn skips_garbage_and_decodes_legacy_text() {
        let mut codec = IrcCodec::default();
        let mut buf = BytesMut::from(&b":\r\n@x\r\nPRIVMSG #c :caf\xe9\r\n"[..]);
        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.params[1], "café");
    }

    #[test]
    fn discards_over_long_lines() {
        let mut codec = IrcCodec::default();
        let mut buf = BytesMut::from(vec![b'A'; MAX_LINE_LEN + 1].as_slice());
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert!(buf.is_empty(), "buffer must not grow without bound");
        buf.extend_from_slice(b"still the long line\r\nPING :ok\r\n");
        assert_eq!(decode_all(&mut codec, &mut buf), ["PING"]);
    }

    #[test]
    fn encodes_with_crlf() {
        let mut buf = BytesMut::new();
        IrcCodec::default()
            .encode("PONG :x".to_owned(), &mut buf)
            .unwrap();
        assert_eq!(&buf[..], b"PONG :x\r\n");
    }
}
