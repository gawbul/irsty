# Scripting

Irsty runs scripts written in [Luau](https://luau.org), the Lua dialect with a built-in sandbox ([ADR 0008](adr/0008-luau-scripting.md)). Scripts can react to what happens in buffers, add commands, print, and set timers. They can't read or write files, open connections, or run programs, and they act for you on the network only if you allow it.

Working examples are in [`examples/scripts`](https://github.com/gawbul/irsty/tree/main/examples/scripts), and a test loads and exercises every one of them.

## Loading scripts

List scripts in the config to load them at startup:

```toml
[[script]]
path = "scripts/mentions.luau"   # relative to the config file's directory

[[script]]
path = "~/irsty/slap.luau"
allow = ["send"]                 # may send messages and run commands

[[script]]
path = "scripts/experiment.luau"
enabled = false
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `path` | path | required | The script. Relative paths are relative to the directory holding `config.toml`; `~/` is your home directory. |
| `allow` | list | `[]` | Capabilities: `"send"` lets it send messages (`irsty.say`) and run commands (`irsty.exec`). |
| `enabled` | boolean | `true` | Load it. |

From inside Irsty:

| Command | What it does |
|---|---|
| `/script` (or `/script list`) | List loaded scripts, what they may do and the commands they add. |
| `/script load <path> [-allow send]` | Load a script until Irsty quits (add it to the config to keep it). Loading a script with the same name replaces it. |
| `/script unload <name>` | Unload a script. Its commands, handlers and timers go with it. |
| `/script reload [name]` | Load a script (or every script) again from its file, after you edit it. |

`/reload` loads the config's scripts again if `[[script]]` changed. A script's name is its file name without the extension.

## What a script can do

Everything goes through the `irsty` table.

### Events

```lua
irsty.on("message", function(e)
    if e.highlight and not e.own then
        print("mentioned by " .. e.nick .. " in " .. e.buffer)
    end
end)
```

`irsty.on(event, handler)` calls `handler(e)` for each new line of that kind. The events are `message`, `action`, `notice`, `join`, `part`, `quit`, `kick`, `nick`, `mode`, `topic`, `info` and `error`, or `line` for all of them. The handler gets a table:

| Field | What it is |
|---|---|
| `network` | The network's name. |
| `buffer` | The channel or nick; `nil` in a server buffer. |
| `kind` | The event, as above. |
| `nick` | Who: the sender of a message, the person joining, leaving or quitting, the kicker, the old nick in a nick change, or whoever set the topic. `nil` for server lines. |
| `text` | The line as shown, without formatting. |
| `time` | When, in seconds since 1970 (`os.date` formats it). |
| `own` | Whether you said or did it. |
| `highlight` | Whether it mentions you or one of your highlight words. |
| `msgid` | The server's id for the message, if it gave one. |

Only live lines are events. Lines replayed from chat history or a bouncer are not, so a script never answers yesterday's messages. Lines a script's own actions add (its output, and messages it sends) are not handed back to it either, so a script can't loop on itself.

### Commands

```lua
irsty.command("hello", function(args, here)
    print("Hello from " .. (here and here.network or "nowhere") .. ": " .. args)
end)
```

`irsty.command(name, handler)` adds `/name`. The handler gets the rest of the line as one string and a table describing where it was typed (`network`, `buffer`, `kind`), or `nil` before any network. Built-in commands and aliases come first, so a script can't replace `/msg`. Names are letters, digits, `-` and `_`. Script commands appear at the end of `/help` and complete with Tab.

### Output

- `print(...)` writes its arguments, separated by tabs, to the current buffer (never to the terminal).
- `irsty.print(text, place)` writes `text` to a buffer: the current one, or `place = {network = "libera", buffer = "#rust"}` (leave out `buffer` for the server buffer). mIRC formatting codes in `text` work, and everything is sanitised like text from the network.

Output is shown, not sent, and is not written to the logs.

### Acting for you (needs `send`)

- `irsty.say(network, target, text)` sends a message to a channel or nick, as if you had typed it there.
- `irsty.exec(line)` runs a command as if you had typed it, such as `irsty.exec("/me waves")` or `irsty.exec("/join #rust")`.

Both are limited to five every ten seconds per script; past that they raise an error. Without `allow = ["send"]` they raise an error saying how to grant it.

### Timers

```lua
local id = irsty.every(60 * 1000, function() print("another minute") end)
irsty.timer(5000, function() irsty.cancel(id) end)
```

`irsty.timer(ms, f)` calls `f` once after `ms` milliseconds, and `irsty.every(ms, f)` keeps calling it every `ms`; both return an id for `irsty.cancel(id)`. The shortest interval is 10 ms, and a script may have 64 timers.

### Reading state

| Function | Returns |
|---|---|
| `irsty.buffers()` | Every buffer: tables with `network`, `buffer` (`nil` for server buffers), `kind` (`"server"`, `"channel"` or `"query"`) and `unread`. |
| `irsty.current()` | The buffer on screen, as above, or `nil`. |
| `irsty.nicks(network, channel)` | The nicks in a channel, sorted. |
| `irsty.nick(network)` | Your nick on a network, or `nil`. |
| `irsty.version` | Irsty's version. |
| `irsty.events` | The event names. |

These read Irsty's state only while the script is running (in a handler, a command, a timer, or at load). Keep the results, not the functions.

## Limits

Every script runs in a Luau state of its own, in sandbox mode:

- Libraries and globals are read-only. A script's own globals are its own, and scripts can't see each other.
- There is no `io` library, `os` has only `clock`, `date`, `difftime` and `time`, and `require` (which would read files) is removed.
- Each call into a script (a handler, a command, a timer, or loading it) is stopped after 250 ms, so an endless loop can't freeze Irsty. Each script may use 32 MiB of memory.
- A script whose calls fail five times in a row is unloaded. Each error is shown with the script's name.
- Scripts run on the UI thread between frames, so keep handlers short.

## Luau, not Lua

Luau is Lua 5.1 with additions: compound assignment (`n += 1`), `continue`, string interpolation, generalised iteration (`for k, v in t do`), and optional types. Unlike Lua 5.4 it has no `goto` and no integer subtype. See the [Luau syntax guide](https://luau.org/syntax). Scripts written for WeeChat's or irssi's scripting APIs won't run unchanged.
