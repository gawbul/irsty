# Keybindings

Irsty follows irssi by default. Set `keymap = "weechat"` under `[ui]` (or `/set ui.keymap weechat`) for WeeChat's buffer keys. Either way you can rebind any key with `/bind` or the `[keys]` table; see [Customising](#customising).

Keys are written as in `/bind`: `ctrl-n`, `alt-left`, `shift-tab`, `f5`. Letters keep their case, so `alt-B` means Alt+Shift+B.

## Buffers

| Keys | Action | Preset |
|---|---|---|
| Alt+1 to Alt+9, Alt+0 | Go to buffer 1 to 10 | both |
| Alt+Q to Alt+P (the row below the numbers) | Go to buffer 11 to 20 | irssi |
| Alt+J, then two digits | Go to buffer 01 to 99 | WeeChat |
| Ctrl+N, Alt+Right | Next buffer | both |
| Ctrl+P, Alt+Left | Previous buffer | both |
| Alt+Down, F6 | Next buffer | WeeChat |
| Alt+Up, F5 | Previous buffer | WeeChat |
| Alt+A | The buffer with the most important unread activity (highlights first, then messages) | both |
| Alt+< | Back to the buffer shown before | both |
| Alt+> | Forward again | both |
| Alt+/ | Back to the buffer shown before | WeeChat |
| Ctrl+G | The buffer switcher (below) | both |

Buffers are numbered in the list on the left: each network's server buffer, then its channels and queries in the order they were opened.

### The buffer switcher

Ctrl+G opens a box listing every buffer. Type part of a name, in order but not necessarily together (`lbr` finds `#libera-rust`), and the best matches rise to the top; buffers with unread activity win ties. Up and Down (or Ctrl+P and Ctrl+N, or Tab) move the selection, Enter switches, and Esc or Ctrl+G closes it.

## Scrolling

| Keys | Action |
|---|---|
| Page Up, Page Down | Scroll back or forward a page; back past the oldest line fetches older history from servers that keep it |
| Alt+Page Up, Alt+Page Down | Scroll back or forward three lines |
| Alt+Home | The oldest lines |
| Alt+End, Ctrl+End | The newest lines |
| Mouse wheel | Scroll three lines (only with `[ui] mouse = true`) |

While you are scrolled back, `-- more --` appears in the status bar and new lines don't move the view. Sending a message jumps back to the bottom.

## Editing

| Keys | Action |
|---|---|
| Enter | Send the line, or run the command |
| Tab, Shift+Tab | Complete, or cycle through matches (below) |
| Left, Right, Ctrl+F | Move one character |
| Ctrl+B | Move one character left (WeeChat; in irssi it types bold, below) |
| Ctrl+Left, Ctrl+Right, Alt+B, Alt+F | Move one word |
| Home, End, Ctrl+A, Ctrl+E | Move to the start or end |
| Backspace, Ctrl+H | Delete the character before the cursor |
| Delete, Ctrl+D | Delete the character under the cursor |
| Ctrl+W, Alt+Backspace | Cut the previous word |
| Alt+D | Cut the next word |
| Ctrl+U | Cut to the start of the line |
| Ctrl+K | Cut to the end of the line |
| Ctrl+Y | Paste the last cut text |
| Ctrl+T | Swap the characters around the cursor |
| Up, Down, Ctrl+Up, Ctrl+Down | Browse input history (your unsent line comes back when you go past the newest entry) |

Movement and deletion work on whole characters as you see them, so accented letters, emoji and flags are never split.

### Completion

Tab completes the word before the cursor; pressing it again cycles through the other matches, and Shift+Tab goes backwards. Any other key accepts the match.

- **Nicks:** people in the current channel, those who spoke most recently first, then everyone else alphabetically (in a query, the other person). A nick at the start of the line gets `: ` after it, as in irssi.
- **Channels:** words starting with `#` or `&` complete to channels you are in.
- **Commands:** `/wh` completes to `/whois`, `/whowas` and `/who`, and your aliases.
- **Arguments:** `/set` completes setting names (and `irssi` or `weechat` for `ui.keymap`), `/connect`, `/disconnect` and `/server` complete network names, `/server` its subcommands, `/window` its words, `/help` command names, and `/bind` action names.

### Pasting several lines

A paste with more than one line doesn't go into the input. Irsty asks first: **Enter** (or Y) sends the lines as one multi-line message where the server supports it (`draft/multiline`, as Ergo does), and as a message per line otherwise (the prompt says which), **J** joins them into the input as one line to edit, and **Esc** (or N) cancels. Pasted lines are always sent as text, never run as commands, so pasting `/quit` sends the words "/quit". This needs a terminal with bracketed paste, which almost all have.

### Formatting

Type mIRC formatting into your messages. In the input line each code shows as its letter in reverse video (**B** for bold, **C** for colour and so on), and it is sent as typed.

| irssi | WeeChat | Code |
|---|---|---|
| Ctrl+B | Ctrl+C, B | Bold |
| Ctrl+C | Ctrl+C, C | Colour: follow with a number, `4` red, or `4,1` red on black |
| | Ctrl+C, D | Hex colour: follow with `RRGGBB` |
| Ctrl+] | Ctrl+C, I | Italic |
| Ctrl+_ | Ctrl+C, _ | Underline |
| Ctrl+^ | Ctrl+C, S | Strikethrough |
| Ctrl+V | Ctrl+C, V | Reverse |
| Ctrl+O | Ctrl+C, O | Back to plain text |

irssi's keys type the control byte they are named after (Ctrl+B is byte 2, bold), as irssi does. Most terminals send Ctrl+], Ctrl+^ and Ctrl+_ the same way as Ctrl+5, Ctrl+6 and Ctrl+7, so either works.

## Other

| Keys | Action |
|---|---|
| Ctrl+L | Redraw the screen |
| Ctrl+Z | Suspend Irsty (resume with `fg`); the terminal is restored while suspended |
| Mouse click in the buffer list | Switch buffer (only with `[ui] mouse = true`) |

Ctrl+C never quits Irsty: it types a colour code (irssi) or starts a formatting code (WeeChat). Use `/quit` to exit.

## Customising

`/bind` lists every binding in effect. `/bind <key> <action>` changes one, `/bind <key>` shows one, `/bind -reset <key>` goes back to the preset's binding, and `/unbind <key>` makes a key do nothing. Changes are saved to `config.toml` and work at once.

In the config, the same goes in `[keys]`:

```toml
[ui]
keymap = "weechat"

[keys]
"ctrl-g" = "none"        # free Ctrl+G
"alt-x" = "switcher"
"f12" = "buffer_42"
```

A binding Irsty doesn't understand is reported when it starts, and skipped.

### Actions

| Action | Does |
|---|---|
| `send` | Send the line or run the command |
| `complete`, `complete_back` | Complete, or cycle through matches |
| `switcher` | Open the buffer switcher |
| `next_buffer`, `prev_buffer` | Next or previous buffer |
| `next_active` | The buffer with the most important activity |
| `back`, `forward` | Through the buffers you have shown |
| `buffer_1` to `buffer_99` | A buffer by number |
| `jump` | Wait for two digits naming a buffer |
| `page_up`, `page_down` | Scroll a page |
| `scroll_up`, `scroll_down` | Scroll three lines |
| `scroll_top`, `scroll_bottom` | The oldest or newest lines |
| `history_prev`, `history_next` | Input history |
| `left`, `right`, `word_left`, `word_right`, `home`, `end` | Move the cursor |
| `backspace`, `delete` | Delete a character |
| `kill_word_back`, `kill_word_forward`, `kill_to_start`, `kill_to_end` | Cut text |
| `yank` | Paste the last cut |
| `transpose` | Swap the characters around the cursor |
| `bold`, `colour`, `hex_colour`, `italic`, `underline`, `strikethrough`, `monospace`, `reverse`, `reset` | Type a formatting code |
| `format_prefix` | Wait for a letter naming a formatting code (WeeChat's Ctrl+C) |
| `redraw` | Redraw the screen |
| `suspend` | Suspend Irsty |
| `none` | Nothing |
