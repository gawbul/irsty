# Commands

Type commands into the input line. Anything not starting with `/` is sent to the current channel or query. Start a line with `//` to send a message that begins with a slash.

A test checks this file against the command table in `crates/irsty-tui/src/command.rs`, so it can't fall out of date. `/help` lists everything; `/help <command>` shows one command's usage and other names. Tab completes command names, and their arguments where it can (see [KEYBINDINGS.md](KEYBINDINGS.md#completion)).

## Talking

| Command | Usage | What it does |
|---|---|---|
| `/join` (`/j`) | `/join <#channel>[,<#channel>...] [key[,key...]]` | Join one or more channels, then switch to the first. |
| `/part` (`/leave`) | `/part [#channel] [reason]` | Leave a channel; the current one if none is given. The buffer stays open so you can read it; `/close` removes it. |
| `/msg` (`/m`) | `/msg <target> <text>` | Send a message to a nick or channel. Long text is split at word boundaries to fit the line limit. |
| `/query` (`/q`) | `/query <nick> [text]` | Open a private conversation and switch to it, optionally sending a first message. |
| `/me` | `/me <action>` | Send an action (CTCP ACTION) to the current channel or query. |
| `/notice` | `/notice <target> <text>` | Send a notice. |
| `/ctcp` | `/ctcp <target> <command> [arguments]` | Send a CTCP request such as `VERSION`, `PING` or `TIME`. Replies show in the server buffer. |
| `/nick` | `/nick <newnick>` | Change your nick on the current network. |
| `/away` | `/away [message]` | Mark yourself away on the current network, or back if no message is given. |

## Finding out about people and channels

Replies appear in the buffer where you typed the command, as one readable block.

| Command | Usage | What it does |
|---|---|---|
| `/whois` (`/wi`) | `/whois <nick> [nick]` | Who someone is: their host, real name, account, server, channels and away message. Give the nick twice (`/whois bob bob`) to ask their own server, which also knows their idle time. |
| `/whowas` | `/whowas <nick>` | Who last used a nick that is now gone. |
| `/who` | `/who [mask]` | Users matching a mask, or everyone in the current channel. |
| `/names` | `/names [#channel]` | Who is in a channel, ranked as in the nick list. |
| `/topic` (`/t`) | `/topic [#channel] [new topic]` | Show the topic, or set it. |
| `/list` | `/list [arguments]` | Channels on the network, with user counts and topics. Large networks have tens of thousands; most servers accept filters such as `/list >100`. |

## Running a channel

These act on the current channel.

| Command | Usage | What it does |
|---|---|---|
| `/mode` | `/mode [target] <modes> [arguments]` | Change modes of the current channel, another channel or yourself (`/mode +i`). With no modes, shows them. |
| `/op` | `/op <nick> [nick...]` | Give operator status. Several nicks are packed into as few `MODE` lines as the server allows. |
| `/deop` | `/deop <nick> [nick...]` | Take operator status away. |
| `/voice` | `/voice <nick> [nick...]` | Give voice. |
| `/devoice` | `/devoice <nick> [nick...]` | Take voice away. |
| `/kick` (`/k`) | `/kick <nick> [reason]` | Remove someone from the channel. |
| `/ban` | `/ban [nick|mask]` | Ban a mask, or a nick by their host (`*!*@host`) when it's known, else `nick!*@*`. With nothing, list the bans. |
| `/unban` | `/unban <mask>` | Remove a ban. |
| `/invite` | `/invite <nick> [#channel]` | Invite someone to a channel, the current one by default. |

## Buffers

| Command | Usage | What it does |
|---|---|---|
| `/window` (`/buffer`, `/w`) | `/window <number>\|next\|prev\|close\|list` | Go to a buffer by its number in the list, step through them, close the current one, or list them. Ctrl+G opens a fuzzy switcher instead. |
| `/close` (`/wc`) | `/close [reason]` | Close the current buffer, leaving the channel first if you are in it. Server buffers can't be closed. |
| `/clear` | `/clear` | Clear the current buffer's scrollback. Logs are untouched. |

## Networks

| Command | Usage | What it does |
|---|---|---|
| `/connect` | `/connect [network]` | Connect a network that is down, or reconnect one that is up. The current network by default. |
| `/disconnect` | `/disconnect [network] [reason]` | Quit a network and stay disconnected until `/connect`. |
| `/server` (`/network`) | `/server add\|del\|update\|list\|[network] ...` | Manage networks in your config, or connect one, as `/connect`. See below. |
| `/raw` (`/quote`) | `/raw <line>` | Send a raw IRC line to the current network. The copy shown in the server buffer hides passwords (`PASS`, `OPER`, `AUTHENTICATE` payloads, and NickServ-style `IDENTIFY` and `REGISTER`). |

### `/server`

```
/server add <network> <host>[:port] [options]
/server del <network> [host]
/server update <network> [options]
/server list
```

- `add` adds a server to a network, creating the network if it is new; a new network needs `-nick`. The new network connects straight away.
- `del` removes a whole network (disconnecting it), or one server from it.
- `update` changes a network's options and reconnects it if it is connected.

Options: `-nick <nick>[,<alt>...]`, `-autojoin <#a>[,<#b>...]`, `-sasl plain|scram-sha-256|external|none`, `-account <name>`, `-keyring <entry>` (the SASL password's entry in the OS keyring), and for `add` only, `-notls` and `-insecure` (don't verify the certificate). IPv6 addresses go in brackets: `[2001:db8::1]:6697`.

Passwords are never typed into Irsty: store them in the keyring first (see [CONFIG.md](CONFIG.md#secrets)) and pass `-keyring`. For example, from scratch:

```
/server add libera irc.libera.chat -nick gawbul,gawbul_ -autojoin #irsty -sasl scram-sha-256 -account gawbul -keyring libera
```

Changes are written to `config.toml` straight away, keeping your comments and layout.

## Ignores, highlights, aliases and settings

These change `config.toml` and take effect at once.

| Command | Usage | What it does |
|---|---|---|
| `/ignore` | `/ignore [mask [messages\|notices\|ctcps\|events\|invites...] [-network name] [-channel #c]]` | Hide someone. A bare nick means `nick!*@*`; wildcards work. Without kinds, everything is hidden; events are joins, parts, quits, nick changes and kicks. With no arguments, list ignores. Irsty still tracks ignored users, so nick lists stay right. |
| `/unignore` | `/unignore <mask>` | Stop ignoring a mask. |
| `/highlight` (`/hilight`) | `/highlight [word \| -regex pattern]` | Treat lines with a word (whole word, any case) or a regular expression as highlights, like your nick. With no arguments, list them. |
| `/unhighlight` | `/unhighlight <word or pattern>` | Remove a highlight. |
| `/alias` | `/alias [name [/command ...[; /command ...]]]` | Define an alias, show one, or list them. See below. |
| `/unalias` | `/unalias <name>` | Remove an alias. |
| `/set` | `/set [setting [value]] \| /set -reset <setting>` | List settings, show one, change one, or reset it to the default. |
| `/bind` | `/bind [key [action]] \| /bind -reset <key>` | List key bindings, show one, change one, or go back to the preset's. Keys and actions are in [KEYBINDINGS.md](KEYBINDINGS.md#customising). |
| `/unbind` | `/unbind <key>` | Make a key do nothing (the same as `/bind <key> none`). |

### Aliases

```
/alias j /join $*
/alias hi /msg $channel Hello from $nick! ; /me waves
```

In an alias, `$*` is everything typed after it, `$1` to `$9` single words, and `$nick`, `$channel` and `$network` the current nick, channel or query, and network. Separate several commands with `;`. Built-in commands always win over aliases of the same name, and aliases may use other aliases up to eight deep.

### Settings

`/set` changes `scrollback`, `logging.enabled`, `logging.dir`, `ui.mouse`, `ui.nicklist`, `ui.timestamp_format` and `ui.keymap` (see [CONFIG.md](CONFIG.md)). Booleans accept `on`/`off`, `true`/`false` and `yes`/`no`. Logging and mouse changes take effect when Irsty restarts; the rest apply at once.

## Leaving

| Command | Usage | What it does |
|---|---|---|
| `/quit` (`/exit`) | `/quit [reason]` | Quit every network, flush logs and exit. |
| `/help` | `/help [command]` | List commands, or show one command's usage. |

Messages to services bots (NickServ, ChanServ, Q and similar) that carry a password, such as `/msg NickServ IDENTIFY ...`, are sent as typed but shown and logged as `IDENTIFY <redacted>`.
