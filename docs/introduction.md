# Irsty

A modern, keyboard-driven terminal IRC client written in Rust. It is a spiritual successor to irssi and WeeChat: fast, readable out of the box, scriptable, with IRCv3 throughout and good support for bouncers.

```text
Irsty: a terminal IRC client in Rust | https://github.com/gawbul/irsty
 1.libera        │09:08        <-- │ bob has left #irsty (lunch)                     @gawbul
 2.  #irsty      │09:09      alice │ gawbul: are you there?                          +alice
 3.  frank       │09:11      alice │ shall we tag a release today, or wait for the    carol
                 │                 │ docs?  👍 2  🎉 1                                dave_
                 │09:12      carol │ ↪ alice: shall we tag a release today, …         eve
                 │                 │ alice: after the docs land
                 │09:13        eve │ ↪ an earlier message  replying to something
                 │                 │ long gone
                 │09:14      dave_ │ release checklist:
                 │                 │ - docs
                 │                 │ - changelog
[08:59] [gawbul] [2:libera/#irsty] [Act: 3] [Lag: 1.2s]  eve and carol are typing…
[#irsty]
```

## Install

On macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/gawbul/irsty/releases/latest/download/irsty-installer.sh | sh
```

With Homebrew, on macOS and Linux:

```sh
brew install gawbul/tap/irsty
```

On Windows, in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/gawbul/irsty/releases/latest/download/irsty-installer.ps1 | iex"
```

With Rust installed, `cargo install irsty` builds it (you also need a C and C++ compiler, for the bundled SQLite and Luau). The [releases](https://github.com/gawbul/irsty/releases) also have archives for each platform, with the man page and shell completions inside; [Packaging](PACKAGING.md) says where they go.

## Start

Run `irsty` and add a network from inside it:

```text
/server add libera irc.libera.chat -nick yournick,yournick_ -autojoin #irsty
```

That connects, joins `#irsty`, and writes your config file. Type `/help` for commands and `/quit` to leave; Alt+1, Alt+2 and so on switch buffers. With a screen reader, start it as `irsty --plain` ([Accessibility](ACCESSIBILITY.md)).

## What it does

- **Familiar keys:** irssi's by default, or WeeChat's, and any key can be rebound ([Key bindings](KEYBINDINGS.md)).
- **IRCv3 throughout:** chat history, read markers, typing indicators, replies, reactions, multi-line messages, `echo-message` and more, and soju's and ZNC's bouncer extensions.
- **Safe by default:** TLS verified against your system's certificates with plain-English explanations when it fails, secrets in your keyring or password manager rather than the config, and text from the network never able to send escape sequences to your terminal.
- **Search, split windows and layouts:** `/search` over everything Irsty has logged, panes side by side or stacked, and a marker where you stopped reading ([Commands](COMMANDS.md)).
- **Themes:** dark, light and high-contrast presets, every element restylable, and colours fitted to what your terminal can show, `NO_COLOR` included ([Configuration](CONFIG.md#theme)).
- **Scripting:** Luau scripts in a sandbox of their own ([Scripting](SCRIPTING.md)).
- **Accessible:** a plain, line-by-line mode for screen readers and braille displays, and an accessible mode for the full-screen interface ([Accessibility](ACCESSIBILITY.md)).
- **DCC:** files and chats directly with others, including irssi users, with resume and passive DCC ([Commands](COMMANDS.md#files-and-direct-chats-dcc)).

Irsty is free software under the Apache License 2.0. The source, issues and releases are on [GitHub](https://github.com/gawbul/irsty).
