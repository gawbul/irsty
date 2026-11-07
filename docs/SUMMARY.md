# Summary

[Irsty](introduction.md)

# Using Irsty

- [Configuration](CONFIG.md)
- [Commands](COMMANDS.md)
- [Key bindings](KEYBINDINGS.md)
- [Scripting](SCRIPTING.md)
- [Accessibility](ACCESSIBILITY.md)

# Packaging and development

- [Packaging and releases](PACKAGING.md)
- [Architecture](ARCHITECTURE.md)
- [Dependency versions](VERSIONS.md)
- [Decision records](adr/README.md)
  - [0001: Our own sans-IO protocol crate](adr/0001-own-protocol-crate.md)
  - [0002: Unidirectional state and a single update step](adr/0002-unidirectional-state.md)
  - [0003: Our own scrollback and input widgets](adr/0003-bespoke-scrollback-and-input.md)
  - [0004: Secrets come from the keyring or a command, never plaintext by default](adr/0004-secrets.md)
  - [0005: Untrusted text is sanitised once, at ingest](adr/0005-untrusted-text.md)
  - [0006: MSRV trails stable by two releases](adr/0006-msrv-policy.md)
  - [0007: OSC 52 and OSC 777 before native clipboard and notifications](adr/0007-terminal-protocols-first.md)
  - [0008: Luau for scripting](adr/0008-luau-scripting.md)
  - [0009: Licensed under Apache-2.0 only](adr/0009-apache-2-licence.md)
