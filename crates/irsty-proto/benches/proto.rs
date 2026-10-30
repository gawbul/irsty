//! Baselines for the hot paths: every incoming line is parsed, and every displayed
//! line is formatted.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use irsty_proto::format::StyledText;
use irsty_proto::{Message, MessageRef};

const PRIVMSG: &str =
    ":nick!~user@host.example.com PRIVMSG #channel :Hello, this is a fairly typical chat message.";
const TAGGED: &str = "@time=2026-10-30T12:34:56.789Z;msgid=63E1033A051D4B41B1AB1FA3CF4B243E;account=nick;+draft/reply=abc \
                      :nick!~user@host.example.com PRIVMSG #channel :Hello, this is a fairly typical chat message.";
const FORMATTED: &str = "\x02bold\x02 normal \x0304,12red on blue\x03 \x1ditalic\x1d \x04FF8000hex\x0f and some more plain text";

fn parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for (name, line) in [("privmsg", PRIVMSG), ("tagged", TAGGED)] {
        group.throughput(Throughput::Bytes(line.len() as u64));
        group.bench_function(format!("{name}/borrowed"), |b| {
            b.iter(|| MessageRef::parse(black_box(line)));
        });
        group.bench_function(format!("{name}/owned"), |b| {
            b.iter(|| Message::parse(black_box(line)));
        });
    }
    group.finish();
}

fn serialise(c: &mut Criterion) {
    let msg = Message::parse(TAGGED).unwrap_or_default();
    c.bench_function("serialise/tagged", |b| b.iter(|| black_box(&msg).to_wire()));
}

fn format(c: &mut Criterion) {
    let mut group = c.benchmark_group("format");
    group.throughput(Throughput::Bytes(FORMATTED.len() as u64));
    group.bench_function("styled", |b| {
        b.iter(|| StyledText::parse(black_box(FORMATTED)));
    });
    group.bench_function("plain_text", |b| {
        b.iter(|| StyledText::parse(black_box(PRIVMSG)));
    });
    group.finish();
}

criterion_group!(benches, parse, serialise, format);
criterion_main!(benches);
