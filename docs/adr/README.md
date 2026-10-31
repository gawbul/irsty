# Architecture decision records

Each record is short: the context, the decision, and its consequences. Once accepted, a record is not edited; a new record supersedes it.

| No. | Title | Status |
|---|---|---|
| [0001](0001-own-protocol-crate.md) | Our own sans-IO protocol crate | Accepted |
| [0002](0002-unidirectional-state.md) | Unidirectional state and a single update step | Accepted |
| [0003](0003-bespoke-scrollback-and-input.md) | Our own scrollback and input widgets | Accepted |
| [0004](0004-secrets.md) | Secrets come from the keyring or a command, never plaintext by default | Accepted |
| [0005](0005-untrusted-text.md) | Untrusted text is sanitised once, at ingest | Accepted |
| [0006](0006-msrv-policy.md) | MSRV trails stable by two releases | Accepted |
| [0007](0007-terminal-protocols-first.md) | OSC 52 and OSC 777 before native clipboard and notifications | Accepted |
| [0008](0008-luau-scripting.md) | Luau for scripting | Accepted |
| [0009](0009-apache-2-licence.md) | Licensed under Apache-2.0 only | Accepted |
