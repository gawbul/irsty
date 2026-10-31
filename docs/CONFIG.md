# Configuration

Irsty reads a single `config.toml`:

| Platform | Default location |
|---|---|
| Linux, macOS | `$XDG_CONFIG_HOME/irsty/config.toml` (usually `~/.config/irsty/config.toml`) |
| Windows | `%APPDATA%\irsty\config\config.toml` |

A relative `XDG_CONFIG_HOME` is ignored, as the XDG spec requires. Logs and other data still use the platform data directory (see `[logging]`).

Use `irsty --config <PATH>` to read another file. Unknown keys are errors, so typos never pass silently. Error messages give a line and column but never quote the file, which may contain secrets.

## A complete example

```toml
scrollback = 10000

[logging]
enabled = true
# dir = "~/irclogs"

[[network]]
name = "libera"
nicks = ["gawbul", "gawbul_"]
username = "gawbul"
realname = "Steve"
autojoin = ["#irsty", "#rust"]
perform = ["MODE $nick +i"]
monitor = ["a_friend"]

[[network.servers]]
host = "irc.libera.chat"
port = 6697

[[network.servers]]
host = "irc.eu.libera.chat"

[network.sasl]
mechanism = "scram-sha-256"
account = "gawbul"
password = { keyring = "libera" }

[[network]]
name = "tor-example"
nicks = ["gawbul"]

# Tor's SOCKS port; the onion address is resolved by Tor, not locally.
[network.proxy]
host = "127.0.0.1"
port = 9050

[[network.servers]]
host = "the-network-onion-address.onion"
tls = false

[[network]]
name = "bouncer"
nicks = ["gawbul"]

[[network.servers]]
host = "bnc.example.net"
port = 6697
password = { command = ["pass", "irc/bouncer"] }
```

## Top level

| Key | Type | Default | Meaning |
|---|---|---|---|
| `scrollback` | integer | `10000` | Lines kept in memory per buffer. Older lines are dropped from memory but stay in the logs. |

## `[logging]`

Plain-text logs, one file per buffer, at `<dir>/<network>/<buffer>.log` (the server buffer is `-server.log`). Names are lowercased and made safe for the file system, and files are created with mode `0600`. Each line is an ISO 8601 UTC timestamp followed by irssi-style text. mIRC formatting codes are kept; every other control character is replaced with a visible symbol, so viewing a log with `cat` is safe.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` | Write logs. |
| `dir` | path | platform data directory + `/logs` | Where logs go. Linux: `~/.local/share/irsty/logs`; macOS: `~/Library/Application Support/irsty/logs`. |

Irsty's own diagnostic log is `irsty.log` in the platform data directory. Set `IRSTY_LOG` to change its filter, for example `IRSTY_LOG=irsty_core=debug`. It never contains passwords.

## `[ui]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `mouse` | boolean | `false` | Capture the mouse: the wheel scrolls and clicking the buffer list switches buffers. Off by default so your terminal's own text selection keeps working. |
| `nicklist` | boolean | `true` | Show the nick list in channels when the terminal is at least 100 columns wide. |
| `timestamp_format` | string | `"%H:%M"` | [`strftime`](https://docs.rs/jiff/latest/jiff/fmt/strftime/index.html) format for timestamps, in your local time zone. |

## `[[network]]`

Repeat for each network.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | Display name; also the log directory name. Must be unique (case-insensitively). |
| `nicks` | list of strings | required | Nicks to try in order. If all are taken, Irsty appends a number to the first. |
| `username` | string | first nick | Username (ident). |
| `realname` | string | first nick | Real name (gecos). |
| `autojoin` | list of strings | `[]` | Channels to join after every registration. Channels you were in when a connection dropped are rejoined too. |
| `monitor` | list of strings | `[]` | Nicks to watch. Irsty says in the server buffer (and their query, if open) when they come online or leave. Needs a server with `MONITOR`; Irsty tells you if it has none. |
| `perform` | list of strings | `[]` | Raw IRC lines (as with `/raw`) sent after every registration, before autojoin. `$nick` becomes your current nick, for example `"MODE $nick +i"` or `"PRIVMSG ChanServ :OP #channel $nick"`. Lines are checked when the config loads. Prefer SASL to identifying here: a line holding a password gets the plaintext warning, though it is redacted from logs and debug output. |
| `autoconnect` | boolean | `true` | Connect at startup. |
| `sasl` | table | none | See below. |
| `client_cert` | table | none | See below. |
| `proxy` | table | none | A SOCKS5 proxy; see below. |

### `[[network.servers]]`

At least one per network. Irsty resolves the host and races its IPv6 and IPv4 addresses (Happy Eyeballs, RFC 8305): it starts the next address every 250 ms or as soon as one fails, so a broken IPv6 route costs a quarter of a second rather than a timeout. On failure, Irsty moves to the next server and backs off exponentially (2 s, then doubling up to 5 minutes, with jitter); a connection that stays up for a minute resets the backoff.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `host` | string | required | Hostname or IP address. IPv6 addresses may be written with or without brackets (`"[2001:db8::1]"`). |
| `port` | integer | `6697` | Port. |
| `tls` | boolean | `true` | Use TLS. |
| `tls_verify` | boolean | `true` | Verify the server's certificate against the operating system's trust store. Setting this to `false` makes the connection open to interception; Irsty warns in red every time it connects. Only use it for servers you control with self-signed certificates. |
| `password` | secret | none | Server password (`PASS`), typically for bouncers. |

When a certificate is rejected, the server buffer says why in plain words and what to do. For example, if the certificate doesn't cover the host you configured, Irsty lists the names it does cover, so you can switch to one of them:

```
Disconnected (cannot connect to irc.example.net:6697 (TLS): the certificate is for irc.example.com, not irc.example.net. If one of those is the same server, use it as the host); reconnecting in 1s
```

The full explanation appears once per server; retries show `same certificate problem as before` and keep backing off, so Irsty reconnects by itself if the operators fix their certificate.

### `[network.sasl]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `mechanism` | `"scram-sha-256"`, `"plain"` or `"external"` | required | `scram-sha-256` proves your password without sending it, and checks the server knows it too; prefer it where the server offers it. `plain` sends the account and password (inside TLS). `external` uses the TLS client certificate (CertFP) and needs `client_cert`. |
| `account` | string | none | Account name; required for `scram-sha-256` and `plain`. |
| `password` | secret | none | Required for `scram-sha-256` and `plain`. |

If SASL fails or the server doesn't offer it, Irsty says so in the server buffer and carries on connecting without authentication. If the server lists its mechanisms and yours isn't among them, Irsty doesn't try another one: falling back from SCRAM to PLAIN on a hostile server would hand it your password.

To use CertFP, connect once with a password and the certificate configured, send `/msg NickServ CERT ADD` (the exact command depends on the network's services), then switch `mechanism` to `"external"`.

### `[network.proxy]`

Connects through a SOCKS5 proxy, such as Tor or `ssh -D`. Host names are passed to the proxy unresolved, so your DNS lookups don't reveal where you connect.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `host` | string | required | Proxy host name or IP address. |
| `port` | integer | `1080` | Proxy port. Tor uses `9050` (or `9150` for Tor Browser). |
| `username` | string | none | For proxies that need a login. Requires `password`. |
| `password` | secret | none | Requires `username`. |

### `[network.client_cert]`

| Key | Type | Meaning |
|---|---|---|
| `cert` | path | PEM certificate chain. |
| `key` | path | PEM private key (PKCS #8, PKCS #1 or SEC1). May be the same file as `cert`. |

## Secrets

Passwords are never bare strings ([ADR 0004](adr/0004-secrets.md)). A secret is one of:

```toml
password = { keyring = "libera" }               # the OS keyring, recommended
password = { command = ["pass", "irc/libera"] } # the first line a command prints
password = { plain = "hunter2" }                # plaintext: allowed, with a warning
```

- **`keyring`**: the macOS Keychain, or the Secret Service (GNOME Keyring, KWallet) on Linux, under service `irsty` and the name given. On macOS, add one with `security add-generic-password -s irsty -a libera -w`. On Linux, use `secret-tool store --label "irsty libera" service irsty username libera`.
- **`command`**: runs the program directly (not through a shell) with standard input closed, and uses the first line of its output. It has two minutes to finish, so GPG can prompt for a passphrase. Works with `pass`, `gopass`, `op read`, `security find-generic-password -w` and so on.
- **`plain`**: kept for convenience. Irsty prints a warning in the network's server buffer at every start.

Secrets are resolved once at startup. If one can't be read, that network doesn't connect and the reason appears in its server buffer; other networks are unaffected.
