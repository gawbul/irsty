# 0004: Secrets come from the keyring or a command, never plaintext by default

- Status: accepted
- Date: 2026-10-30

## Context

IRC credentials (SASL passwords, server passwords, bouncer passwords) end up in dotfiles, which are often committed to public repositories.

## Decision

A secret in the config is one of these:

```toml
password = { keyring = "irsty/libera" }          # OS keychain via the keyring crate
password = { command = ["pass", "irc/libera"] }  # runs once, stdout trimmed
password = { plain = "hunter2" }                 # allowed, with a warning at every startup
```

A bare string is a config error, and the error message explains the three options. Secrets are held in a type whose `Debug` and `Display` print `<redacted>`. They are scrubbed from `/raw` output (`AUTHENTICATE`, `PASS`, NickServ `IDENTIFY`) and from tracing.

## Consequences

- There is a little friction for new users. The default config and `docs/CONFIG.md` show the keyring form first.
- Secrets stored by command work with `pass`, `op`, `security` and anything else that prints to stdout.
