//! Luau scripting (ADR 0008).
//!
//! Each script runs in a Luau state of its own, in sandbox mode: globals and
//! libraries are read-only, there is no `io`, and `require` (which would read
//! files) is removed, so a script can reach nothing outside Irsty's API. Every
//! call into a script is interrupted after [`TIME_LIMIT`], each state is
//! limited to [`MEMORY_LIMIT`], and a script that fails [`MAX_ERRORS`] times
//! in a row is unloaded.
//!
//! Scripts never touch the client. They queue [`Action`]s, which the app
//! carries out after the script returns, and read state through functions
//! that borrow it only for the length of the call (`Lua::scope`). Acting for
//! the user (`irsty.say`, `irsty.exec`) needs the `send` capability and is
//! rate-limited. See `docs/SCRIPTING.md` for the API.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use irsty_core::config::Capability;
use irsty_core::state::{BufferId, BufferKind, Line, LineKind, State};
use mlua::{Function, Lua, Table, Value, VmState};

/// Most memory one script's state may use.
pub const MEMORY_LIMIT: usize = 32 * 1024 * 1024;
/// Longest one call into a script may run.
pub const TIME_LIMIT: Duration = Duration::from_millis(250);
/// Errors in a row before a script is unloaded.
pub const MAX_ERRORS: u32 = 5;
/// Largest script file read.
const MAX_SOURCE: u64 = 1024 * 1024;
/// Sends (`say` and `exec`) allowed per [`SEND_WINDOW`].
const SEND_BURST: usize = 5;
const SEND_WINDOW: Duration = Duration::from_secs(10);
/// Shortest timer interval.
const MIN_TIMER: Duration = Duration::from_millis(10);
/// Most timers one script may have.
const MAX_TIMERS: usize = 64;
/// Most actions one call may queue.
const MAX_ACTIONS: usize = 256;

/// Events a script can listen for: a line of each kind, or `line` for all.
pub const EVENTS: &[&str] = &[
    "line", "message", "action", "notice", "join", "part", "quit", "kick", "nick", "mode", "topic",
    "info", "error",
];

/// Something a script asked for, done by the app once the script returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Show text: in the current buffer, or a buffer by network and name
    /// (`None` for its server buffer).
    Print {
        /// Where.
        place: Option<(String, Option<String>)>,
        /// What, with mIRC formatting allowed.
        text: String,
    },
    /// Send a message (needs `send`).
    Say {
        /// The network's name.
        network: String,
        /// The channel or nick.
        target: String,
        /// The text.
        text: String,
    },
    /// Run a command as if typed (needs `send`).
    Exec(String),
    /// A script failed; show why.
    Error(String),
}

/// What a script may read while it runs.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// Client state.
    pub state: &'a State,
    /// The buffer on screen.
    pub current: Option<BufferId>,
}

#[derive(Debug)]
struct Timer {
    id: u64,
    due: Instant,
    every: Option<Duration>,
}

/// Per-script bookkeeping, kept in the Lua state's app data.
#[derive(Debug, Default)]
struct Host {
    actions: Vec<Action>,
    timers: Vec<Timer>,
    next_timer: u64,
    deadline: Option<Instant>,
    send: bool,
    sends: VecDeque<Instant>,
}

/// A loaded script.
struct Script {
    name: String,
    path: Option<PathBuf>,
    allow: Vec<Capability>,
    lua: Lua,
    api: Table,
    hooks: Table,
    commands: Table,
    timers: Table,
    errors: u32,
}

/// What `/script list` shows about a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Its name (the file name without `.luau` or `.lua`).
    pub name: String,
    /// Where it was loaded from.
    pub path: Option<PathBuf>,
    /// Capabilities granted.
    pub allow: Vec<Capability>,
    /// Commands it added.
    pub commands: Vec<String>,
}

/// Every loaded script, and which lines they have seen.
#[derive(Default)]
pub struct Scripts {
    scripts: Vec<Script>,
    seen: HashMap<BufferId, u64>,
}

impl std::fmt::Debug for Scripts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scripts")
            .field(
                "loaded",
                &self
                    .scripts
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// A script's name from its path: the file name without its extension.
pub fn name_of(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| "script".into(), |s| s.to_string_lossy().into_owned())
}

/// The first line of an error, which is enough to say what went wrong.
fn brief(error: &mlua::Error) -> String {
    let text = error.to_string();
    text.lines().next().unwrap_or_default().to_owned()
}

impl Scripts {
    /// Whether any script is loaded.
    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    /// The loaded scripts.
    pub fn list(&self) -> Vec<Summary> {
        self.scripts
            .iter()
            .map(|s| Summary {
                name: s.name.clone(),
                path: s.path.clone(),
                allow: s.allow.clone(),
                commands: command_names(&s.commands),
            })
            .collect()
    }

    /// Commands scripts have added, sorted.
    pub fn commands(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .scripts
            .iter()
            .flat_map(|s| command_names(&s.commands))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Whether a script added the command `name`.
    pub fn has_command(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.scripts
            .iter()
            .any(|s| s.commands.contains_key(name.as_str()).unwrap_or(false))
    }

    /// Reads and loads a script file (replacing one with the same name).
    pub fn load_file(
        &mut self,
        path: &Path,
        allow: &[Capability],
        ctx: Context<'_>,
    ) -> Result<Vec<Action>, String> {
        let size = std::fs::metadata(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        if size > MAX_SOURCE {
            return Err(format!("{}: larger than 1 MiB", path.display()));
        }
        let source =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.load(&name_of(path), &source, Some(path), allow, ctx)
    }

    /// Loads a script from source (replacing one with the same name) and
    /// runs its top level. Returns what it asked for.
    pub fn load(
        &mut self,
        name: &str,
        source: &str,
        path: Option<&Path>,
        allow: &[Capability],
        ctx: Context<'_>,
    ) -> Result<Vec<Action>, String> {
        let script = new_script(name, path, allow).map_err(|e| format!("{name}: {}", brief(&e)))?;
        self.unload(name);
        self.prime(ctx.state);
        self.scripts.push(script);
        let index = self.scripts.len() - 1;
        let chunk_name = format!("={name}");
        let (actions, failed) = self.call(index, ctx, |lua, _| {
            lua.load(source).set_name(chunk_name).exec()
        });
        if let Some(error) = failed {
            self.scripts.remove(index);
            return Err(error);
        }
        Ok(actions)
    }

    /// Unloads a script. Returns whether it was loaded.
    pub fn unload(&mut self, name: &str) -> bool {
        let before = self.scripts.len();
        self.scripts.retain(|s| s.name != name);
        self.scripts.len() != before
    }

    /// Unloads every script.
    pub fn clear(&mut self) {
        self.scripts.clear();
    }

    /// Where a loaded script came from, and what it may do, to load it again.
    pub fn origin(&self, name: &str) -> Option<(PathBuf, Vec<Capability>)> {
        let script = self.scripts.iter().find(|s| s.name == name)?;
        Some((script.path.clone()?, script.allow.clone()))
    }

    /// Treats every line now in `state` as seen, so loading a script doesn't
    /// replay the scrollback to it.
    pub fn prime(&mut self, state: &State) {
        self.seen = state.buffers().map(|b| (b.id, b.total)).collect();
    }

    /// Marks lines added to `state` since `before` (buffer totals) as seen:
    /// lines the scripts' own actions caused, which they mustn't see again.
    pub fn skip_added(&mut self, state: &State, before: &HashMap<BufferId, u64>) {
        for buffer in state.buffers() {
            if before.get(&buffer.id) != Some(&buffer.total) {
                self.seen.insert(buffer.id, buffer.total);
            }
        }
    }

    /// Hands lines added since the last call to the scripts listening for
    /// them. Lines from history are skipped.
    pub fn dispatch_lines(&mut self, ctx: Context<'_>) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.scripts.is_empty() {
            return actions;
        }
        let mut new: Vec<(BufferId, u64)> = Vec::new();
        for buffer in ctx.state.buffers() {
            let seen = self.seen.insert(buffer.id, buffer.total).unwrap_or(0);
            let oldest = (buffer.total + 1).saturating_sub(buffer.lines().len() as u64);
            for seq in seen.max(oldest.saturating_sub(1)) + 1..=buffer.total {
                new.push((buffer.id, seq));
            }
        }
        self.seen.retain(|id, _| ctx.state.buffer(*id).is_some());
        for (buffer, seq) in new {
            let Some(line) = ctx.state.buffer(buffer).and_then(|b| b.line(seq)) else {
                continue;
            };
            if line.history {
                continue;
            }
            let kind = kind_name(line.kind);
            for index in 0..self.scripts.len() {
                let wanted = self.scripts.get(index).is_some_and(|s| {
                    [kind, "line"]
                        .iter()
                        .any(|e| s.hooks.contains_key(*e).unwrap_or(false))
                });
                if !wanted {
                    continue;
                }
                let (mut out, _) = self.call(index, ctx, |lua, script| {
                    let event = event_table(lua, ctx, buffer, line)?;
                    for name in [kind, "line"] {
                        if let Ok(Value::Table(list)) = script.hooks.get::<Value>(name) {
                            for handler in list.sequence_values::<Function>() {
                                handler?.call::<()>(event.clone())?;
                            }
                        }
                    }
                    Ok(())
                });
                actions.append(&mut out);
            }
            self.drop_failing(&mut actions);
        }
        actions
    }

    /// Runs a script command with its arguments.
    pub fn run_command(&mut self, name: &str, args: &str, ctx: Context<'_>) -> Vec<Action> {
        let name = name.to_lowercase();
        let Some(index) = self
            .scripts
            .iter()
            .position(|s| s.commands.contains_key(name.as_str()).unwrap_or(false))
        else {
            return Vec::new();
        };
        let (mut actions, _) = self.call(index, ctx, |lua, script| {
            let handler: Function = script.commands.get(name.as_str())?;
            let place = place_table(lua, ctx, ctx.current)?;
            handler.call::<()>((args, place))
        });
        self.drop_failing(&mut actions);
        actions
    }

    /// When the next timer is due, if any.
    pub fn next_due(&self) -> Option<Instant> {
        self.scripts
            .iter()
            .filter_map(|s| {
                s.lua
                    .app_data_ref::<Host>()
                    .and_then(|h| h.timers.iter().map(|t| t.due).min())
            })
            .min()
    }

    /// Runs the timers due by `now`.
    pub fn fire_due(&mut self, now: Instant, ctx: Context<'_>) -> Vec<Action> {
        let mut actions = Vec::new();
        for index in 0..self.scripts.len() {
            let due: Vec<u64> = self
                .scripts
                .get(index)
                .and_then(|s| s.lua.app_data_mut::<Host>())
                .map(|mut host| {
                    let ids: Vec<u64> = host
                        .timers
                        .iter()
                        .filter(|t| t.due <= now)
                        .map(|t| t.id)
                        .collect();
                    // Repeating timers are rescheduled from now, so a slow
                    // loop never queues a burst of catch-up calls.
                    host.timers.retain_mut(|t| {
                        if t.due > now {
                            return true;
                        }
                        match t.every {
                            Some(every) => {
                                t.due = now + every;
                                true
                            }
                            None => false,
                        }
                    });
                    ids
                })
                .unwrap_or_default();
            for id in due {
                let (mut out, _) = self.call(index, ctx, |_, script| {
                    let handler: Option<Function> = script.timers.get(id)?;
                    let once = script
                        .lua
                        .app_data_ref::<Host>()
                        .is_some_and(|h| !h.timers.iter().any(|t| t.id == id));
                    if once {
                        script.timers.set(id, Value::Nil)?;
                    }
                    match handler {
                        Some(f) => f.call::<()>(()),
                        None => Ok(()),
                    }
                });
                actions.append(&mut out);
            }
        }
        self.drop_failing(&mut actions);
        actions
    }

    /// Unloads scripts that keep failing, saying so.
    fn drop_failing(&mut self, actions: &mut Vec<Action>) {
        self.scripts.retain(|s| {
            if s.errors >= MAX_ERRORS {
                actions.push(Action::Error(format!(
                    "Script {} unloaded after {MAX_ERRORS} errors in a row",
                    s.name
                )));
                false
            } else {
                true
            }
        });
    }

    /// Calls into script `index` under its time limit, with the state
    /// queries available. Returns its actions, and the error if it failed
    /// (also among the actions).
    fn call(
        &mut self,
        index: usize,
        ctx: Context<'_>,
        f: impl FnOnce(&Lua, &Script) -> mlua::Result<()>,
    ) -> (Vec<Action>, Option<String>) {
        let Some(script) = self.scripts.get_mut(index) else {
            return (Vec::new(), None);
        };
        if let Some(mut host) = script.lua.app_data_mut::<Host>() {
            host.deadline = Some(Instant::now() + TIME_LIMIT);
        }
        let result = script.lua.scope(|scope| {
            let api = &script.api;
            api.set_readonly(false);
            let installed = (|| {
                api.set(
                    "buffers",
                    scope.create_function(move |lua, ()| buffers_table(lua, ctx))?,
                )?;
                api.set(
                    "current",
                    scope.create_function(move |lua, ()| place_table(lua, ctx, ctx.current))?,
                )?;
                api.set(
                    "nicks",
                    scope.create_function(move |lua, (network, channel): (String, String)| {
                        nicks_table(lua, ctx, &network, &channel)
                    })?,
                )?;
                api.set(
                    "nick",
                    scope.create_function(move |_, network: String| {
                        Ok(ctx
                            .state
                            .networks()
                            .find(|n| n.name == network)
                            .map(|n| n.nick.clone()))
                    })?,
                )
            })();
            api.set_readonly(true);
            installed?;
            f(&script.lua, script)
        });
        let mut actions = script
            .lua
            .app_data_mut::<Host>()
            .map(|mut host| {
                host.deadline = None;
                std::mem::take(&mut host.actions)
            })
            .unwrap_or_default();
        match result {
            Ok(()) => {
                script.errors = 0;
                (actions, None)
            }
            Err(error) => {
                script.errors += 1;
                let text = format!("Script {}: {}", script.name, brief(&error));
                actions.push(Action::Error(text.clone()));
                (actions, Some(text))
            }
        }
    }
}

fn command_names(commands: &Table) -> Vec<String> {
    let mut names: Vec<String> = commands
        .pairs::<String, Value>()
        .filter_map(|pair| pair.ok().map(|(k, _)| k))
        .collect();
    names.sort();
    names
}

fn kind_name(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Message => "message",
        LineKind::Action => "action",
        LineKind::Notice => "notice",
        LineKind::Join => "join",
        LineKind::Part => "part",
        LineKind::Quit => "quit",
        LineKind::Kick => "kick",
        LineKind::Nick => "nick",
        LineKind::Mode => "mode",
        LineKind::Topic => "topic",
        LineKind::Info => "info",
        LineKind::Error => "error",
    }
}

/// `{network, buffer, kind}` for a buffer; `buffer` is nil for a server
/// buffer.
fn place_table(lua: &Lua, ctx: Context<'_>, buffer: Option<BufferId>) -> mlua::Result<Value> {
    let Some(buf) = buffer.and_then(|b| ctx.state.buffer(b)) else {
        return Ok(Value::Nil);
    };
    let t = lua.create_table()?;
    if let Some(net) = ctx.state.network(buf.network) {
        t.set("network", net.name.as_str())?;
    }
    if buf.kind != BufferKind::Server {
        t.set("buffer", buf.name.as_str())?;
    }
    t.set(
        "kind",
        match buf.kind {
            BufferKind::Server => "server",
            BufferKind::Channel => "channel",
            BufferKind::Query => "query",
        },
    )?;
    Ok(Value::Table(t))
}

fn buffers_table(lua: &Lua, ctx: Context<'_>) -> mlua::Result<Table> {
    let list = lua.create_table()?;
    for buffer in ctx.state.buffers() {
        if let Value::Table(t) = place_table(lua, ctx, Some(buffer.id))? {
            t.set("unread", buffer.unread)?;
            list.push(t)?;
        }
    }
    Ok(list)
}

fn nicks_table(lua: &Lua, ctx: Context<'_>, network: &str, channel: &str) -> mlua::Result<Table> {
    let list = lua.create_table()?;
    let Some(chan) = ctx
        .state
        .networks()
        .find(|n| n.name == network)
        .and_then(|n| n.channel(channel))
    else {
        return Ok(list);
    };
    let mut nicks: Vec<&str> = chan.members().map(|m| m.nick.as_str()).collect();
    nicks.sort_unstable_by_key(|n| n.to_lowercase());
    for nick in nicks {
        list.push(nick)?;
    }
    Ok(list)
}

/// The table a line event handler receives.
fn event_table(lua: &Lua, ctx: Context<'_>, buffer: BufferId, line: &Line) -> mlua::Result<Table> {
    let event = match place_table(lua, ctx, Some(buffer))? {
        Value::Table(t) => t,
        _ => lua.create_table()?,
    };
    event.set("kind", kind_name(line.kind))?;
    event.set("text", line.text.to_plain())?;
    event.set("time", line.time.as_second())?;
    event.set("highlight", line.highlight)?;
    if let Some(nick) = &line.sender {
        event.set("nick", nick.as_str())?;
        let own = ctx
            .state
            .buffer(buffer)
            .and_then(|b| ctx.state.network(b.network))
            .is_some_and(|n| n.is_me(nick));
        event.set("own", own)?;
    } else {
        event.set("own", false)?;
    }
    if let Some(msgid) = &line.msgid {
        event.set("msgid", msgid.as_str())?;
    }
    Ok(event)
}

/// Queues an action, within [`MAX_ACTIONS`] per call.
fn queue(lua: &Lua, action: Action) -> mlua::Result<()> {
    let mut host = lua
        .app_data_mut::<Host>()
        .ok_or_else(|| mlua::Error::runtime("no host"))?;
    if host.actions.len() >= MAX_ACTIONS {
        return Err(mlua::Error::runtime("too many actions in one call"));
    }
    host.actions.push(action);
    Ok(())
}

/// Checks the `send` capability and the send rate.
fn may_send(lua: &Lua) -> mlua::Result<()> {
    let mut host = lua
        .app_data_mut::<Host>()
        .ok_or_else(|| mlua::Error::runtime("no host"))?;
    if !host.send {
        return Err(mlua::Error::runtime(
            "this script may not send; grant it with allow = [\"send\"] in its [[script]] entry",
        ));
    }
    let now = Instant::now();
    while host
        .sends
        .front()
        .is_some_and(|&t| now.duration_since(t) >= SEND_WINDOW)
    {
        host.sends.pop_front();
    }
    if host.sends.len() >= SEND_BURST {
        return Err(mlua::Error::runtime(format!(
            "sending too fast (at most {SEND_BURST} every {} seconds)",
            SEND_WINDOW.as_secs()
        )));
    }
    host.sends.push_back(now);
    Ok(())
}

/// A place from a Lua table `{network = ..., buffer = ...}`.
fn place_from(value: Option<Table>) -> mlua::Result<Option<(String, Option<String>)>> {
    let Some(t) = value else {
        return Ok(None);
    };
    let network: String = t.get("network")?;
    let buffer: Option<String> = t.get("buffer")?;
    Ok(Some((network, buffer)))
}

fn add_timer(lua: &Lua, timers: &Table, ms: f64, f: Function, repeat: bool) -> mlua::Result<u64> {
    if !ms.is_finite() || ms < 0.0 {
        return Err(mlua::Error::runtime("a timer needs a time in milliseconds"));
    }
    let interval = Duration::from_secs_f64(ms / 1000.0).max(MIN_TIMER);
    let mut host = lua
        .app_data_mut::<Host>()
        .ok_or_else(|| mlua::Error::runtime("no host"))?;
    if host.timers.len() >= MAX_TIMERS {
        return Err(mlua::Error::runtime(format!(
            "at most {MAX_TIMERS} timers per script"
        )));
    }
    host.next_timer += 1;
    let id = host.next_timer;
    host.timers.push(Timer {
        id,
        due: Instant::now() + interval,
        every: repeat.then_some(interval),
    });
    drop(host);
    timers.set(id, f)?;
    Ok(id)
}

/// Builds a script's sandboxed state and its `irsty` table.
fn new_script(name: &str, path: Option<&Path>, allow: &[Capability]) -> mlua::Result<Script> {
    let lua = Lua::new();
    lua.set_memory_limit(MEMORY_LIMIT)?;
    lua.set_app_data(Host {
        send: allow.contains(&Capability::Send),
        ..Host::default()
    });
    lua.set_interrupt(|lua| {
        let late = lua
            .app_data_ref::<Host>()
            .and_then(|h| h.deadline)
            .is_some_and(|d| Instant::now() > d);
        if late {
            return Err(mlua::Error::runtime(format!(
                "stopped after {} ms",
                TIME_LIMIT.as_millis()
            )));
        }
        Ok(VmState::Continue)
    });
    let globals = lua.globals();
    // `require` reads files; nothing outside the API is reachable.
    globals.set("require", Value::Nil)?;

    let hooks = lua.create_table()?;
    let commands = lua.create_table()?;
    let timers = lua.create_table()?;
    let api = lua.create_table()?;
    api.set("version", env!("CARGO_PKG_VERSION"))?;
    api.set("events", EVENTS.to_vec())?;
    add_registration(&lua, &api, &hooks, &commands)?;
    add_output(&lua, &api, &globals)?;
    add_timers(&lua, &api, &timers)?;
    globals.set("irsty", api.clone())?;
    lua.sandbox(true)?;
    Ok(Script {
        name: name.to_owned(),
        path: path.map(Path::to_owned),
        allow: allow.to_vec(),
        lua,
        api,
        hooks,
        commands,
        timers,
        errors: 0,
    })
}

/// `irsty.on` and `irsty.command`.
fn add_registration(lua: &Lua, api: &Table, hooks: &Table, commands: &Table) -> mlua::Result<()> {
    let event_hooks = hooks.clone();
    api.set(
        "on",
        lua.create_function(move |lua, (event, f): (String, Function)| {
            if !EVENTS.contains(&event.as_str()) {
                return Err(mlua::Error::runtime(format!(
                    "no event {event:?}; events are {}",
                    EVENTS.join(", ")
                )));
            }
            let list = if let Value::Table(t) = event_hooks.get::<Value>(event.as_str())? {
                t
            } else {
                let t = lua.create_table()?;
                event_hooks.set(event.as_str(), t.clone())?;
                t
            };
            list.push(f)
        })?,
    )?;
    let script_commands = commands.clone();
    api.set(
        "command",
        lua.create_function(move |_, (name, f): (String, Function)| {
            let name = name.trim_start_matches('/').to_lowercase();
            let valid = !name.is_empty()
                && name.len() <= 32
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid {
                return Err(mlua::Error::runtime(format!(
                    "{name:?} is not a command name (letters, digits, - and _)"
                )));
            }
            script_commands.set(name, f)
        })?,
    )?;
    Ok(())
}

/// `irsty.print`, the global `print`, `irsty.say` and `irsty.exec`.
fn add_output(lua: &Lua, api: &Table, globals: &Table) -> mlua::Result<()> {
    api.set(
        "print",
        lua.create_function(|lua, (text, place): (String, Option<Table>)| {
            queue(
                lua,
                Action::Print {
                    place: place_from(place)?,
                    text,
                },
            )
        })?,
    )?;
    // The global print goes to the current buffer, not the terminal.
    globals.set(
        "print",
        lua.create_function(|lua, args: mlua::Variadic<Value>| {
            let text = args
                .iter()
                .map(|v| v.to_string().unwrap_or_else(|_| "?".into()))
                .collect::<Vec<_>>()
                .join("\t");
            queue(lua, Action::Print { place: None, text })
        })?,
    )?;
    api.set(
        "say",
        lua.create_function(|lua, (network, target, text): (String, String, String)| {
            may_send(lua)?;
            queue(
                lua,
                Action::Say {
                    network,
                    target,
                    text,
                },
            )
        })?,
    )?;
    api.set(
        "exec",
        lua.create_function(|lua, line: String| {
            if !line.starts_with('/') {
                return Err(mlua::Error::runtime(
                    "exec runs commands, which start with /",
                ));
            }
            may_send(lua)?;
            queue(lua, Action::Exec(line))
        })?,
    )?;
    Ok(())
}

/// `irsty.timer`, `irsty.every` and `irsty.cancel`.
fn add_timers(lua: &Lua, api: &Table, timers: &Table) -> mlua::Result<()> {
    let once = timers.clone();
    api.set(
        "timer",
        lua.create_function(move |lua, (ms, f): (f64, Function)| {
            add_timer(lua, &once, ms, f, false)
        })?,
    )?;
    let every = timers.clone();
    api.set(
        "every",
        lua.create_function(move |lua, (ms, f): (f64, Function)| {
            add_timer(lua, &every, ms, f, true)
        })?,
    )?;
    let cancel = timers.clone();
    api.set(
        "cancel",
        lua.create_function(move |lua, id: u64| {
            if let Some(mut host) = lua.app_data_mut::<Host>() {
                host.timers.retain(|t| t.id != id);
            }
            cancel.set(id, Value::Nil)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use irsty_core::event::{NetEvent, NetworkId};
    use irsty_proto::Message;

    const NET: NetworkId = NetworkId(1);

    fn state() -> State {
        let mut s = State::new(100);
        s.add_network(NET, "libera", "me", Vec::new());
        s.apply(NET, NetEvent::Registered { nick: "me".into() });
        feed(&mut s, ":me!u@h JOIN #c");
        feed(&mut s, ":srv 353 me = #c :me @alice bob");
        s
    }

    fn feed(s: &mut State, line: &str) {
        s.apply(NET, NetEvent::Message(Message::parse(line).unwrap()));
    }

    fn ctx(state: &State) -> Context<'_> {
        Context {
            state,
            current: state.find_buffer(NET, "#c"),
        }
    }

    fn printed(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Print { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn events_commands_and_queries() {
        let mut s = state();
        let mut scripts = Scripts::default();
        let actions = scripts
            .load(
                "greet",
                r##"
                local count = 0
                irsty.on("message", function(e)
                    count += 1
                    irsty.print(e.nick .. " in " .. e.buffer .. ": " .. e.text .. (e.own and " (me)" or ""))
                end)
                irsty.on("join", function(e) print("join", e.nick) end)
                irsty.command("count", function(args, here)
                    irsty.print(count .. " " .. args .. " " .. here.buffer)
                end)
                local names = {}
                for _, b in irsty.buffers() do table.insert(names, b.kind .. ":" .. (b.buffer or b.network)) end
                print(table.concat(names, ","), #irsty.nicks("libera", "#c"), irsty.nick("libera"), irsty.current().buffer)
                "##,
                None,
                &[],
                ctx(&s),
            )
            .unwrap();
        assert_eq!(printed(&actions), ["server:libera,channel:#c\t3\tme\t#c"]);
        assert_eq!(scripts.commands(), ["count"]);
        assert!(scripts.has_command("COUNT"));

        // The scrollback from before loading isn't replayed.
        assert!(scripts.dispatch_lines(ctx(&s)).is_empty());
        feed(&mut s, ":alice!a@h PRIVMSG #c :hello");
        feed(&mut s, ":carol!c@h JOIN #c");
        feed(&mut s, ":me!u@h PRIVMSG #c :hi");
        let actions = scripts.dispatch_lines(ctx(&s));
        assert_eq!(
            printed(&actions),
            ["alice in #c: hello", "join\tcarol", "me in #c: hi (me)"]
        );
        assert!(scripts.dispatch_lines(ctx(&s)).is_empty(), "each line once");
        let actions = scripts.run_command("count", "a b", ctx(&s));
        assert_eq!(printed(&actions), ["2 a b #c"]);
        assert!(scripts.unload("greet"));
        assert!(!scripts.has_command("count"));
    }

    #[test]
    fn history_is_not_an_event() {
        let mut s = state();
        s.apply(
            NET,
            NetEvent::Caps {
                enabled: vec!["batch".into()],
                values: Vec::new(),
            },
        );
        let mut scripts = Scripts::default();
        scripts
            .load(
                "h",
                r#"irsty.on("line", function(e) print(e.text) end)"#,
                None,
                &[],
                ctx(&s),
            )
            .unwrap();
        feed(&mut s, "BATCH +h chathistory #c");
        feed(&mut s, "@batch=h :alice!a@h PRIVMSG #c :old news");
        feed(&mut s, "BATCH -h");
        assert!(printed(&scripts.dispatch_lines(ctx(&s))).is_empty());
    }

    #[test]
    fn the_sandbox_holds() {
        let s = state();
        let mut scripts = Scripts::default();
        let actions = scripts
            .load(
                "probe",
                r#"
                print(require == nil, io == nil, os.execute == nil, os.exit == nil, getmetatable("").__index == string)
                x = 1 -- a script's own globals are its own
                print(x)
                print(pcall(function() string.upper = nil end))
                "#,
                None,
                &[],
                ctx(&s),
            )
            .unwrap();
        let out = printed(&actions);
        assert_eq!(out[0], "true\ttrue\ttrue\ttrue\ttrue");
        assert_eq!(out[1], "1");
        assert!(
            out[2].starts_with("false"),
            "libraries are read-only: {out:?}"
        );
    }

    #[test]
    fn limits_stop_runaway_scripts() {
        let s = state();
        let mut scripts = Scripts::default();
        let started = Instant::now();
        let error = scripts
            .load("spin", "while true do end", None, &[], ctx(&s))
            .unwrap_err();
        assert!(error.contains("stopped after 250 ms"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let error = scripts
            .load(
                "hog",
                "local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1000000) .. i end",
                None,
                &[],
                ctx(&s),
            )
            .unwrap_err();
        assert!(error.contains("memory"), "{error}");
        assert!(scripts.is_empty(), "failed loads leave nothing behind");

        // Five errors in a row unload a script.
        scripts
            .load(
                "flaky",
                r#"irsty.command("boom", function() error("nope") end)"#,
                None,
                &[],
                ctx(&s),
            )
            .unwrap();
        for _ in 0..MAX_ERRORS - 1 {
            let out = scripts.run_command("boom", "", ctx(&s));
            assert!(
                matches!(&out[0], Action::Error(e) if e.contains("nope")),
                "{out:?}"
            );
        }
        let out = scripts.run_command("boom", "", ctx(&s));
        assert!(matches!(out.last(), Some(Action::Error(e)) if e.contains("unloaded")));
        assert!(scripts.is_empty());
    }

    #[test]
    fn sending_needs_the_capability_and_is_rate_limited() {
        let s = state();
        let mut scripts = Scripts::default();
        let source = r##"irsty.command("go", function(n)
            for i = 1, tonumber(n) do irsty.say("libera", "#c", "hi " .. i) end
        end)"##;
        scripts.load("quiet", source, None, &[], ctx(&s)).unwrap();
        let out = scripts.run_command("go", "1", ctx(&s));
        assert!(
            matches!(&out[0], Action::Error(e) if e.contains("may not send")),
            "{out:?}"
        );

        scripts
            .load(
                "loud",
                &source.replace("go", "speak"),
                None,
                &[Capability::Send],
                ctx(&s),
            )
            .unwrap();
        let out = scripts.run_command("speak", "7", ctx(&s));
        let said = out
            .iter()
            .filter(|a| matches!(a, Action::Say { .. }))
            .count();
        assert_eq!(said, SEND_BURST, "queued before the limit");
        assert!(matches!(out.last(), Some(Action::Error(e)) if e.contains("too fast")));
        let error = scripts
            .load(
                "bad",
                r#"irsty.exec("join #x")"#,
                None,
                &[Capability::Send],
                ctx(&s),
            )
            .unwrap_err();
        assert!(error.contains("start with /"));
    }

    #[test]
    fn timers() {
        let s = state();
        let mut scripts = Scripts::default();
        scripts
            .load(
                "tick",
                r#"
                local n = 0
                local id = irsty.every(1000, function() n += 1 print("tick " .. n) end)
                irsty.timer(500, function() print("once") end)
                irsty.command("stop", function() irsty.cancel(id) end)
                "#,
                None,
                &[],
                ctx(&s),
            )
            .unwrap();
        let due = scripts.next_due().unwrap();
        assert!(due <= Instant::now() + Duration::from_millis(600));
        let later = Instant::now() + Duration::from_secs(2);
        assert_eq!(
            printed(&scripts.fire_due(later, ctx(&s))),
            ["tick 1", "once"]
        );
        let much_later = later + Duration::from_secs(2);
        assert_eq!(printed(&scripts.fire_due(much_later, ctx(&s))), ["tick 2"]);
        scripts.run_command("stop", "", ctx(&s));
        assert!(scripts.next_due().is_none());
    }

    #[test]
    fn bad_scripts_are_reported() {
        let s = state();
        let mut scripts = Scripts::default();
        for source in [
            "this is not luau",
            r#"irsty.on("sneeze", function() end)"#,
            r#"irsty.command("no spaces", function() end)"#,
        ] {
            assert!(
                scripts.load("x", source, None, &[], ctx(&s)).is_err(),
                "{source}"
            );
        }
        assert!(scripts.is_empty());
    }

    /// The examples in `examples/scripts` load, and do what they say.
    #[test]
    fn the_examples_work() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/scripts");
        let mut s = state();
        let mut scripts = Scripts::default();
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        assert!(files.len() >= 3);
        for file in &files {
            scripts
                .load_file(file, &[Capability::Send], ctx(&s))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(scripts.commands(), ["clock", "mentions", "slap"]);

        feed(&mut s, ":alice!a@h PRIVMSG #c :me: ping");
        feed(&mut s, ":bob!b@h PRIVMSG #c :unrelated");
        scripts.dispatch_lines(ctx(&s));
        let out = printed(&scripts.run_command("mentions", "", ctx(&s)));
        assert_eq!(out.len(), 1);
        assert!(out[0].ends_with("libera/#c <alice> me: ping"), "{out:?}");

        let out = scripts.run_command("slap", "bob", ctx(&s));
        assert_eq!(
            out,
            [Action::Exec(
                "/me slaps bob around a bit with a large trout".into()
            )]
        );
        scripts.run_command("clock", "", ctx(&s));
        assert!(scripts.next_due().is_none(), "/clock cancels its timers");
    }
}
