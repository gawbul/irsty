# Keybindings

The defaults follow irssi. Configurable bindings, `/bind` and the `keymap = "weechat"` preset arrive in M7.

## Buffers

| Keys | Action |
|---|---|
| Alt+1 to Alt+9, Alt+0 | Go to buffer 1 to 10 |
| Alt+Q to Alt+P (the row below) | Go to buffer 11 to 20 |
| Ctrl+N, Alt+Right | Next buffer |
| Ctrl+P, Alt+Left | Previous buffer |
| Alt+A | Go to the buffer with the most important unread activity (highlights first, then messages) |

Buffers are numbered in the list on the left: each network's server buffer, then its channels and queries in the order they were opened.

## Scrolling

| Keys | Action |
|---|---|
| Page Up, Page Down | Scroll back or forward a page |
| Ctrl+End | Jump to the newest lines |
| Mouse wheel | Scroll three lines (only with `[ui] mouse = true`) |

While you are scrolled back, `-- more --` appears in the status bar and new lines don't move the view. Sending a message jumps back to the bottom.

## Editing

| Keys | Action |
|---|---|
| Enter | Send the line, or run the command |
| Left, Right, Ctrl+B, Ctrl+F | Move one character |
| Ctrl+Left, Ctrl+Right, Alt+B, Alt+F | Move one word |
| Home, End, Ctrl+A, Ctrl+E | Move to the start or end |
| Backspace, Ctrl+H | Delete the character before the cursor |
| Delete, Ctrl+D | Delete the character under the cursor |
| Ctrl+W, Alt+Backspace | Cut the previous word |
| Alt+D | Cut the next word |
| Ctrl+U | Cut to the start of the line |
| Ctrl+K | Cut to the end of the line |
| Ctrl+Y | Paste the last cut text |
| Up, Down | Browse input history (your unsent line comes back when you go past the newest entry) |

Movement and deletion work on whole characters as you see them, so accented letters, emoji and flags are never split. Pasted text with line breaks is joined into one line with spaces; a prompt for multi-line pastes arrives in M7.

## Other

| Keys | Action |
|---|---|
| Ctrl+L | Redraw the screen |
| Ctrl+Z | Suspend Irsty (resume with `fg`); the terminal is restored while suspended |
| Mouse click in the buffer list | Switch buffer (only with `[ui] mouse = true`) |

Ctrl+C does nothing for now: in irssi it starts a colour code, which arrives with formatting input in M8. Use `/quit` to exit.
