# Commands

Type commands into the input line. Anything not starting with `/` is sent to the current channel or query. Start a line with `//` to send a message that begins with a slash.

A test checks this file against the command table in `crates/irsty-tui/src/command.rs`, so it can't fall out of date. The full irssi and WeeChat command set arrives in M6.

| Command | Usage | What it does |
|---|---|---|
| `/join` (`/j`) | `/join <#channel>[,<#channel>...] [key[,key...]]` | Join one or more channels, then switch to the first. |
| `/part` (`/leave`) | `/part [#channel] [reason]` | Leave a channel; the current one if none is given. The buffer stays open so you can read it; `/close` removes it. |
| `/msg` (`/m`) | `/msg <target> <text>` | Send a message to a nick or channel. Long text is split at word boundaries to fit the line limit. |
| `/query` (`/q`) | `/query <nick> [text]` | Open a private conversation and switch to it, optionally sending a first message. |
| `/me` | `/me <action>` | Send an action (CTCP ACTION) to the current channel or query. |
| `/nick` | `/nick <newnick>` | Change your nick on the current network. |
| `/raw` (`/quote`) | `/raw <line>` | Send a raw IRC line to the current network. The copy shown in the server buffer hides passwords (`PASS`, `OPER`, `AUTHENTICATE` payloads, and NickServ-style `IDENTIFY` and `REGISTER`). |
| `/connect` (`/server`) | `/connect [network]` | Connect a network that is down, or reconnect one that is up. The current network by default. |
| `/disconnect` | `/disconnect [network] [reason]` | Quit a network and stay disconnected until `/connect`. |
| `/close` (`/wc`) | `/close [reason]` | Close the current buffer, leaving the channel first if you are in it. Server buffers can't be closed. |
| `/quit` (`/exit`) | `/quit [reason]` | Quit every network, flush logs and exit. |
| `/help` | `/help [command]` | List commands, or show one command's usage. |

Messages to services bots (NickServ, ChanServ, Q and similar) that carry a password, such as `/msg NickServ IDENTIFY ...`, are sent as typed but shown and logged as `IDENTIFY <redacted>`.
