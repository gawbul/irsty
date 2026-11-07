# Roadmap

What Irsty could do next, after v0.1.0. [`PLAN.md`](PLAN.md) records what was planned and built (M0 to M16); this file is the list of candidates for what comes after, roughly in the order they're worth doing. Nothing here is promised. When an item is picked up it becomes a milestone in `PLAN.md`, on its own branch and pull request, and leaves this list.

Sizes are rough: **S** is a day or two, **M** about a week, **L** longer. "Draft" marks an IRCv3 specification that is still a work in progress and may change; Irsty already uses several drafts (chat history, multi-line messages, read markers, reactions), and supporting one means following its changes.

## What IRCv3 gives us, and where Irsty stands

IRCv3 is a set of extensions to IRC, each negotiated as a capability, a message tag or an `ISUPPORT` token, so a client uses what a server offers and loses nothing elsewhere. The full list is at [ircv3.net/irc](https://ircv3.net/irc/).

Irsty already supports capability negotiation and `cap-notify`, message tags, SASL, account tracking (`account-notify`, `account-tag`, `extended-join`), `away-notify`, batches (netsplits collapsed, chat history), `chghost`, `setname`, `echo-message`, `invite-notify`, `labeled-response`, `multi-prefix`, `userhost-in-names`, `server-time`, standard replies, message IDs, `MONITOR`, and these drafts: `chathistory`, `multiline`, `read-marker`, and the `reply`, `react` and `typing` client tags. It also speaks the bouncer extensions of soju (`soju.im/bouncer-networks`) and ZNC (`znc.in/playback`, `znc.in/self-message`).

What's left is below, with everything else Irsty could grow into.

## Next: small, ratified, and visible

These use finished specifications, are mostly small, and fix things people notice.

- **Strict Transport Security (`sts`).** A server can say "use TLS on this port from now on". Irsty would remember that policy per network (for as long as the server asks), upgrade plain connections automatically, and refuse to fall back to plain text while the policy lasts. The brief puts security first, and this is the IRCv3 feature that most directly serves it. **M**
- **WHOX.** `ISUPPORT` already tells Irsty when a server has it, but nothing asks. On joining a channel, a `WHO #channel %tcuhnfar` fills in everyone's account, away state and real name at once, so the nick list dims people who were already away when you joined, and account-based features work for everyone, not just people who speak after you arrive. **S**
- **Bot mode.** Servers mark bots with a user mode (`BOT` in `ISUPPORT`, already parsed) and a `bot` tag on their messages. Irsty would show a bot marker in the nick list and beside their lines, and could let a script's account mark itself as a bot. **S**
- **Joins and parts in history (`draft/event-playback`).** Without it, Ergo replays joins and parts as messages from `HistServ`. With it, history brings them back as real events, shown like live ones. **S**
- **Re-authenticating (SASL 3.2).** Log in again without reconnecting, when services come back after a restart or when you change account. **S**

## Then: friends, and less noise

- **A notify list, with extended monitor.** irssi's notify list, built on `MONITOR` (already used for queries) and `extended-monitor`: `/notify add alice`, a line when a friend comes online or goes offline, and their away state and account changes as they happen. Optionally, a "friends" view in the buffer list. **M**
- **Smart filtering of joins and parts.** WeeChat's best-loved default: in busy channels, hide joins, parts and quits from people who haven't spoken recently, and show them for people who have. One setting, per channel or global. **M**
- **Filters.** WeeChat's `/filter`: hide lines matching a pattern, a nick or a kind of line, per buffer, with a key to show everything for a moment. **M**
- **Composing in your editor.** A key (Ctrl+X Ctrl+E, as in shells) that opens the input in `$EDITOR`, and sends what you write as one multi-line message where the server allows it, or as several lines where it doesn't. **S**

## Later: drafts worth following

Useful, but their specifications are still moving, so each waits until a common server (Ergo, soju, Solanum, InspIRCd) ships it.

- **Message redaction (draft).** Deleting a message you sent, and showing "message deleted" for others' redactions, with `/redact` and a key on a selected line. **M**
- **Channel renaming (draft).** Following a `RENAME`: the buffer, its logs, its search index and any saved layout move with the channel. **S**
- **Metadata (`draft/metadata-2`).** Information people attach to themselves: a display name, pronouns, a status, a homepage. Shown in `/whois` and, if wanted, beside nicks. **M**
- **Account registration (draft).** `/register` instead of messaging NickServ, and a step in the first-run screen for networks that offer it. **M**
- **Away while away (`pre-away`).** Connecting to a bouncer while marked away, so a reconnect doesn't announce you as back. **S**
- **Channel context (`+draft/channel-context`).** A private message from a bot about a channel (a help reply, a game's turn) shown in that channel instead of a query. **S**
- **`no-implicit-names`.** In channels of thousands, skipping the automatic `NAMES` on join and fetching the list only when it's shown. Worth it only after measuring. **S**

## Beyond the protocol

- **More scripting.** Per-script storage that survives restarts (still no file access: a small key-value store the sandbox owns), access to more events (nick list changes, mode changes), and a page of example scripts on the site. **M**
- **Themes.** More presets (Solarized, Catppuccin, Gruvbox), and a theme gallery on the site drawn by the same renders as the front page. **S**
- **Inline images (opt-in).** `PLAN.md` kept this out until Phase 3 was done, and it is. Showing linked images with the kitty, iTerm2 or sixel graphics protocols, off by default: the protocols fight ratatui's redrawing and tmux, and fetching a link tells its host your address. **L**
- **Link titles (opt-in).** Fetching the title of a linked page to show beside it. Off by default for the same privacy reason, with an allow-list of hosts. **M**
- **Performance.** Measuring channels of 10,000 users and scrollback of a million lines, and fixing what that finds. **M**
- **Translations.** The interface's text in other languages. Needs the text gathered in one place first. **L**

## Packaging and releases

- **Publish to the AUR** once registration reopens: the package in `packaging/aur` is ready, and a workflow can follow.
- **Submit to nixpkgs**: `packaging/nix/package.nix` is written for it.
- **winget**, for Windows users without Scoop.
- **Signed Windows binaries**, so SmartScreen stops warning.
- **Build provenance**: dist's build attestations, checked by the Scoop workflow, and immutable releases.
- **Distribution repositories** (Debian, Fedora, Homebrew core) once Irsty has users asking for them.

## Not planned

- **A built-in bouncer, or detaching and reattaching.** soju and ZNC do this well, and Irsty already works well with both; tmux covers the rest.
- **WebSocket transport and WEBIRC.** For browser clients and web gateways, not a terminal client.
- **Web Push.** A bouncer feature for phones.
- **Network icons.** No good way to show one in a terminal.
- **Encrypted messaging (FiSH, OTR).** Their cryptography is weak or abandoned; TLS to the server and a bouncer you trust is the honest answer for now.
- **DCC through a proxy.** DCC connects directly, so it would reveal the address the proxy hides; Irsty refuses it, by design.
