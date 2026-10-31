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

[aliases]
j = "/join $*"
hi = ["/msg $channel Hello from $nick!", "/me waves"]

[highlight]
words = ["irsty"]
regex = ['\bsteve\b']

[keys]
"alt-x" = "switcher"

[[ignore]]
mask = "*!*@spam.example"

[[ignore]]
mask = "chattybot"
what = ["events"]
channels = ["#irsty"]

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

Irsty starts without a config file: the first-run screen shows how to add a network with `/server add`, which creates the file. Commands that change settings (`/server`, `/set`, `/ignore`, `/highlight`, `/alias`, `/bind`) write `config.toml` straight away, keeping your comments, spacing and order. Each change is checked as a whole before it is written, through a temporary file so a crash never leaves half a config, and the file is readable only by you. Passwords are never written by Irsty: `/server` only adds `keyring` references.

## `[aliases]`

Your own commands. Each is a command, or a list of commands run in order:

```toml
[aliases]
j = "/join $*"
hi = ["/msg $channel Hello from $nick!", "/me waves"]
```

In a command, `$*` is everything typed after the alias, `$1` to `$9` single words, and `$nick`, `$channel` and `$network` the current nick, channel or query, and network. Names may use letters, digits, `-` and `_`. Built-in commands always win over an alias with the same name. `/alias` and `/unalias` manage them from inside Irsty.

## `[highlight]`

Lines that mention your nick are always highlights. These add more:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `words` | list of strings | `[]` | Whole words, matched without regard to case. |
| `regex` | list of strings | `[]` | Regular expressions in [Rust `regex` syntax](https://docs.rs/regex/latest/regex/#syntax), matched without regard to case against the text without formatting. Matching always takes linear time, so no pattern can hang Irsty. Use single quotes in TOML so backslashes stay as typed: `'\bsteve\b'`. |

## `[[ignore]]`

Hide people. Repeat for each rule. Ignored users are still tracked, so nick lists stay accurate; only their lines are hidden, and nothing from them opens a query.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `mask` | string | required | `nick!user@host` with `*` and `?` wildcards. A bare nick means `nick!*@*`. Matched with the network's casemapping. |
| `what` | list | everything | Any of `messages` (including actions), `notices`, `ctcps`, `events` (joins, parts, quits, nick changes and kicks) and `invites`. |
| `networks` | list of strings | all | Network names it applies to. |
| `channels` | list of strings | everywhere | Channels it applies to. With any listed, private messages are not hidden. |

`/ignore` and `/unignore` manage them from inside Irsty.

## `[logging]`

Plain-text logs, one file per buffer, at `<dir>/<network>/<buffer>.log` (the server buffer is `-server.log`). Names are lowercased and made safe for the file system, and files are created with mode `0600`. Each line is an ISO 8601 UTC timestamp followed by irssi-style text. mIRC formatting codes are kept; every other control character is replaced with a visible symbol, so viewing a log with `cat` is safe.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` | Write logs. |
| `rotate` | `"never"`, `"daily"` or `"monthly"` | `"never"` | Start a new file each day (`#irsty-2026-10-31.log`) or month (`#irsty-2026-10.log`), by the line's UTC date. |
| `dir` | path | platform data directory + `/logs` | Where logs go. Linux: `~/.local/share/irsty/logs`; macOS: `~/Library/Application Support/irsty/logs`. |

Irsty's own diagnostic log is `irsty.log` in the platform data directory. Set `IRSTY_LOG` to change its filter, for example `IRSTY_LOG=irsty_core=debug`. It never contains passwords.

## `[ui]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `mouse` | boolean | `false` | Capture the mouse: the wheel scrolls and clicking the buffer list switches buffers. Off by default so your terminal's own text selection keeps working. |
| `nicklist` | boolean | `true` | Show the nick list in channels when the terminal is at least 100 columns wide. |
| `timestamp_format` | string | `"%H:%M"` | [`strftime`](https://docs.rs/jiff/latest/jiff/fmt/strftime/index.html) format for timestamps, in your local time zone. |
| `keymap` | `"irssi"` or `"weechat"` | `"irssi"` | Key binding preset. They differ in buffer switching and formatting keys; see [KEYBINDINGS.md](KEYBINDINGS.md). |
| `nick_colours` | boolean | `true` | Give each nick a colour from a hash of its name, so the same person always has the same colour. `[nick_colours]` overrides particular nicks either way. |

Lines that mention your nick or a highlight word show the sender's nick dark on yellow, and links in messages are underlined; `/url` lists and opens them.

## `[nick_colours]`

Colours for particular nicks, by name: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `gray`, `dark_gray`, `light_red`, `light_green`, `light_yellow`, `light_blue`, `light_magenta`, `light_cyan`, `white`. Nicks match without regard to case.

```toml
[nick_colours]
gawbul = "light_cyan"
ChanServ = "dark_gray"
```

## `[notifications]`

Highlights and private messages from others raise a desktop notification through your terminal, by default only while the terminal is not focused. There is at most one a second, and one per buffer every ten seconds. Everything in the text that could act as a terminal control sequence is removed first.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `method` | `"osc777"`, `"osc9"`, `"bell"` or `"none"` | `"osc777"` | How to notify: the OSC 777 or OSC 9 escape sequence (support varies by terminal; try both), the terminal bell, or nothing. Inside tmux the sequence is wrapped so tmux passes it on, which needs `set -g allow-passthrough all` (`on` only passes it while the pane is visible). |
| `when_focused` | boolean | `false` | Notify even while the terminal has focus. |
| `show_text` | boolean | `true` | Include the message; off shows only who and where. |

## `[ctcp]`

Irsty answers CTCP `VERSION`, `PING`, `TIME`, `CLIENTINFO` and `SOURCE` requests, at most three every ten seconds per network so a flood of requests can't get you disconnected. Requests and whether they were answered show in the server buffer.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `replies` | boolean | `true` | Answer at all. |
| `version` | string | `"Irsty <version> (https://github.com/gawbul/irsty)"` | The `VERSION` reply. |
| `time` | boolean | `true` | Answer `TIME`, which reveals your local time and time zone. |

## `[keys]`

Key bindings on top of the preset, as `"key" = "action"`. `"none"` frees a preset key. Keys and actions are listed in [KEYBINDINGS.md](KEYBINDINGS.md#customising); `/bind` and `/unbind` change them from inside Irsty. A binding Irsty doesn't understand is reported at startup and skipped.

```toml
[keys]
"ctrl-g" = "none"
"alt-x" = "switcher"
```

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

Secrets are read when a network connects: at startup, on `/connect`, and when `/server` changes it, so a `pass` or `gopass` prompt never blocks the interface. If one can't be read, that network doesn't connect and the reason appears in its server buffer; other networks are unaffected.
