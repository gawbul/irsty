# Accessibility

Irsty has two ways to make IRC easier to use without seeing a full-screen display well:

- **Plain mode** (`irsty --plain`) for screen readers and braille displays: nothing is drawn, and everything is a line of text.
- **Accessible mode** (`[ui] accessible = true`) for the full-screen UI: one column of text, ASCII only, high contrast, and words for anything that was shown only by colour.

Both work with every command, alias and script.

## Plain mode

```sh
irsty --plain
```

Plain mode writes plain lines of text as things happen, and reads what you type a line at a time, like a shell. Your terminal's own line editing works, and so does your screen reader's review of the terminal, because nothing is ever redrawn or moved.

- New lines in the current buffer are printed as they arrive: `<alice> hello`, `* bob waves`, `carol has joined #rust`.
- From other buffers, only what is for you: mentions and private messages, with where they came from: `libera/#irsty: <dave> steve: look at this`.
- Switching buffers (`/window 2`, `/window next`, `/query alice`) says where you are (`Now in libera/#rust.`) and reads out up to 20 lines you hadn't seen, with their times.
- Lines have no timestamps, which are noise when read aloud, except lines from history and the catch-up after switching, where when matters.
- Replies say who they answer (`(replying to alice) <bob> yes`), and messages that couldn't be sent say why.
- Everything else is as usual: type a message to send it to the current buffer, or a command. `/window list` lists buffers, `/names` lists who is in a channel, and `/help` lists commands. `/quit` or Ctrl+D leaves.

Text is plain: no colours or escape sequences reach the terminal, and the sanitising that protects the full-screen UI applies here too. Desktop notifications are off in plain mode; mentions are announced as lines instead.

## Accessible mode

```toml
[ui]
accessible = true
```

Or `/set ui.accessible on` from inside Irsty. In the full-screen UI this:

- Hides the buffer list and the nick list, so the screen is one column of text. `/window list` and `/names` give the same information, and the status bar names the buffer.
- Draws in ASCII: `|` and `-` instead of box-drawing lines, `re:` for replies, and `...` for someone typing.
- Puts words beside what was shown only by colour or style: `!alice` for a line that mentions you, `(sending)` until the server confirms your message, `>` before the focused pane's title, and `!` after a buffer number in the activity list when it holds a mention.
- Uses the high-contrast theme, unless you choose another preset.

## The high-contrast theme

```toml
[theme]
preset = "high-contrast"
```

The high-contrast theme uses no colours of its own. Everything is drawn in your terminal's own foreground and background, with bold, italic, underline and reverse video for emphasis: bars and the selected buffer are reversed, mentions are reversed and bold, your unconfirmed messages are italic, errors are bold and underlined, and nothing is grey. It works on dark and light backgrounds alike. `accessible = true` picks it unless you set a preset, and any element can still be restyled ([CONFIG.md](CONFIG.md#theme)).

## Also useful

- `NO_COLOR` (or `[theme] colours = "none"`) turns every colour off; bars and highlights stay visible in reverse video.
- `/search` finds what was said without scrolling ([COMMANDS.md](COMMANDS.md)).
- Scripts can announce what matters to you, for example by keeping a list of mentions ([SCRIPTING.md](SCRIPTING.md), `examples/scripts/mentions.luau`).
- The keys can be rebound to whatever suits you ([KEYBINDINGS.md](KEYBINDINGS.md#customising)).
