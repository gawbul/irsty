//! Changing `config.toml` from inside Irsty (`/server`, `/set`, `/ignore`
//! and friends) without losing the user's comments and layout.
//!
//! Every change is made to a copy of the document, which must parse as a
//! valid [`Config`] before it is written. Writes go to a temporary file that
//! replaces the original, so a crash never leaves half a config. Secrets are
//! only ever written as `keyring` or `command` references.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, value};

use crate::config::{Config, ConfigError, IgnoreKind, IgnoreRule};

/// Why a change could not be made.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    /// The change makes no sense (no such network, say).
    #[error("{0}")]
    Invalid(String),
    /// The change would leave an invalid config.
    #[error("{0}")]
    Config(#[from] ConfigError),
    /// The file could not be written.
    #[error("cannot write {path}: {source}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

fn invalid<T>(message: impl Into<String>) -> Result<T, EditError> {
    Err(EditError::Invalid(message.into()))
}

/// The settings `/set` may change, with their types. Secrets are not here, and
/// networks have `/server`.
pub const SETTINGS: &[(&str, SettingKind)] = &[
    ("scrollback", SettingKind::Integer),
    ("logging.enabled", SettingKind::Bool),
    ("logging.dir", SettingKind::Text),
    ("ui.mouse", SettingKind::Bool),
    ("ui.nicklist", SettingKind::Bool),
    ("ui.timestamp_format", SettingKind::Text),
    ("ui.keymap", SettingKind::Text),
];

/// The type of a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    /// `true` or `false` (also `on`, `off`, `yes`, `no`).
    Bool,
    /// A whole number.
    Integer,
    /// Any text.
    Text,
}

/// A server for [`ConfigFile::add_server`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewServer {
    /// Host name or IP address.
    pub host: String,
    /// Port, if not 6697.
    pub port: Option<u16>,
    /// Plain text instead of TLS.
    pub plain: bool,
    /// Don't verify the certificate.
    pub insecure: bool,
}

/// Options `/server add` and `/server update` may set on a network.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkOptions {
    /// Nicks to try.
    pub nicks: Option<Vec<String>>,
    /// Channels to join.
    pub autojoin: Option<Vec<String>>,
    /// SASL mechanism: `plain`, `scram-sha-256`, `external`, or `none` to
    /// remove SASL.
    pub sasl: Option<String>,
    /// SASL account.
    pub account: Option<String>,
    /// Keyring entry holding the SASL password.
    pub keyring: Option<String>,
    /// Command printing the SASL password.
    pub command: Option<Vec<String>>,
}

/// `config.toml`, open for editing.
#[derive(Clone, Debug)]
pub struct ConfigFile {
    path: PathBuf,
    doc: DocumentMut,
}

impl ConfigFile {
    /// Opens `path`, or starts an empty config if it doesn't exist yet. The
    /// file is created on the first change.
    pub fn open(path: &Path) -> Result<(Self, Config), ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let config = Config::parse(&text, path)?;
        // It parsed as TOML above, so this can't fail; fall back to empty anyway.
        let doc = text.parse().unwrap_or_default();
        Ok((
            Self {
                path: path.to_owned(),
                doc,
            },
            config,
        ))
    }

    /// Where the config lives.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file exists yet.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// The document as it would be written.
    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    /// Applies `change` to a copy, checks it, writes it and keeps it. Returns
    /// the new config.
    pub fn edit(
        &mut self,
        change: impl FnOnce(&mut DocumentMut) -> Result<(), EditError>,
    ) -> Result<Config, EditError> {
        let mut doc = self.doc.clone();
        change(&mut doc)?;
        let text = doc.to_string();
        let config = Config::parse(&text, &self.path)?;
        write_atomically(&self.path, &text).map_err(|source| EditError::Write {
            path: self.path.clone(),
            source,
        })?;
        self.doc = doc;
        Ok(config)
    }

    /// Adds a server to a network, creating the network if it is new.
    pub fn add_server(
        &mut self,
        network: &str,
        server: &NewServer,
        options: &NetworkOptions,
    ) -> Result<Config, EditError> {
        self.edit(|doc| {
            let networks = networks_mut(doc)?;
            let index = if let Some(index) = find_network(networks, network) {
                index
            } else {
                let mut table = Table::new();
                table.insert("name", value(network));
                let nicks = options
                    .nicks
                    .clone()
                    .filter(|n| !n.is_empty())
                    .ok_or_else(|| {
                        EditError::Invalid(format!(
                            "a new network needs a nick: add -nick <nick>[,<alt>...] (network {network:?})"
                        ))
                    })?;
                table.insert("nicks", value(string_array(&nicks)));
                networks.push(table);
                networks.len() - 1
            };
            let Some(table) = networks.get_mut(index) else {
                return invalid("no such network");
            };
            apply_options(table, options);
            let servers = table
                .entry("servers")
                .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
            let Some(servers) = servers.as_array_of_tables_mut() else {
                return invalid(format!("network {network:?}: servers is not a list of tables"));
            };
            let mut entry = Table::new();
            entry.insert("host", value(server.host.as_str()));
            if let Some(port) = server.port {
                entry.insert("port", value(i64::from(port)));
            }
            if server.plain {
                entry.insert("tls", value(false));
            }
            if server.insecure {
                entry.insert("tls_verify", value(false));
            }
            servers.push(entry);
            Ok(())
        })
    }

    /// Removes a network, or just one of its servers if `host` is given.
    pub fn remove(&mut self, network: &str, host: Option<&str>) -> Result<Config, EditError> {
        self.edit(|doc| {
            let networks = networks_mut(doc)?;
            let Some(index) = find_network(networks, network) else {
                return invalid(format!("no network named {network:?}"));
            };
            let Some(host) = host else {
                networks.remove(index);
                return Ok(());
            };
            let servers = networks
                .get_mut(index)
                .and_then(|t| t.get_mut("servers"))
                .and_then(Item::as_array_of_tables_mut)
                .ok_or_else(|| EditError::Invalid(format!("network {network:?} has no servers")))?;
            let found = servers.iter().position(|s| {
                s.get("host")
                    .and_then(Item::as_str)
                    .is_some_and(|h| h.eq_ignore_ascii_case(host))
            });
            let Some(found) = found else {
                return invalid(format!("network {network:?} has no server {host:?}"));
            };
            if servers.len() == 1 {
                return invalid(format!(
                    "{host:?} is the only server of {network:?}; remove the network instead"
                ));
            }
            servers.remove(found);
            Ok(())
        })
    }

    /// Changes a network's options.
    pub fn update(&mut self, network: &str, options: &NetworkOptions) -> Result<Config, EditError> {
        self.edit(|doc| {
            let networks = networks_mut(doc)?;
            let Some(table) = find_network(networks, network).and_then(|i| networks.get_mut(i))
            else {
                return invalid(format!("no network named {network:?}"));
            };
            if let Some(nicks) = &options.nicks {
                table.insert("nicks", value(string_array(nicks)));
            }
            apply_options(table, options);
            Ok(())
        })
    }

    /// Sets one of the [`SETTINGS`] from text.
    pub fn set(&mut self, key: &str, text: &str) -> Result<Config, EditError> {
        let Some(&(_, kind)) = SETTINGS.iter().find(|(k, _)| *k == key) else {
            return invalid(format!("unknown setting {key:?}; /set lists them"));
        };
        let parsed: Value = match kind {
            SettingKind::Bool => match text.to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" => true.into(),
                "false" | "off" | "no" => false.into(),
                _ => return invalid(format!("{key} is true or false")),
            },
            SettingKind::Integer => match text.parse::<i64>() {
                Ok(n) => n.into(),
                Err(_) => return invalid(format!("{key} is a whole number")),
            },
            SettingKind::Text => text.into(),
        };
        self.edit(|doc| {
            let (table, leaf) = match key.split_once('.') {
                Some((section, leaf)) => (table_mut(doc.as_table_mut(), section)?, leaf),
                None => (doc.as_table_mut(), key),
            };
            table.insert(leaf, Item::Value(parsed));
            Ok(())
        })
    }

    /// Resets one of the [`SETTINGS`] to its default by removing it.
    pub fn unset(&mut self, key: &str) -> Result<Config, EditError> {
        if !SETTINGS.iter().any(|(k, _)| *k == key) {
            return invalid(format!("unknown setting {key:?}; /set lists them"));
        }
        self.edit(|doc| {
            match key.split_once('.') {
                Some((section, leaf)) => {
                    if let Some(table) = doc.get_mut(section).and_then(Item::as_table_mut) {
                        table.remove(leaf);
                    }
                }
                None => {
                    doc.remove(key);
                }
            }
            Ok(())
        })
    }

    /// Binds a key (`/bind`). The UI checks the key and action names first.
    pub fn bind(&mut self, key: &str, action: &str) -> Result<Config, EditError> {
        self.edit(|doc| {
            table_mut(doc.as_table_mut(), "keys")?.insert(key, value(action));
            Ok(())
        })
    }

    /// Removes a key binding override, restoring the preset's.
    pub fn unbind(&mut self, key: &str) -> Result<Config, EditError> {
        self.edit(|doc| {
            let removed = doc
                .get_mut("keys")
                .and_then(Item::as_table_mut)
                .and_then(|t| t.remove(key));
            if removed.is_none() {
                return invalid(format!("{key} has no binding of its own in the config"));
            }
            Ok(())
        })
    }

    /// Adds or replaces an alias.
    pub fn set_alias(&mut self, name: &str, commands: &[String]) -> Result<Config, EditError> {
        self.edit(|doc| {
            let aliases = table_mut(doc.as_table_mut(), "aliases")?;
            let item = match commands {
                [one] => value(one.as_str()),
                many => value(string_array(many)),
            };
            aliases.insert(name, item);
            Ok(())
        })
    }

    /// Removes an alias.
    pub fn remove_alias(&mut self, name: &str) -> Result<Config, EditError> {
        self.edit(|doc| {
            let removed = doc
                .get_mut("aliases")
                .and_then(Item::as_table_mut)
                .and_then(|t| t.remove(name));
            if removed.is_none() {
                return invalid(format!("no alias named {name:?}"));
            }
            Ok(())
        })
    }

    /// Adds an ignore rule.
    pub fn add_ignore(&mut self, rule: &IgnoreRule) -> Result<Config, EditError> {
        self.edit(|doc| {
            let rules = array_of_tables_mut(doc, "ignore")?;
            let mut table = Table::new();
            table.insert("mask", value(rule.mask.as_str()));
            if rule.what.len() != IgnoreKind::ALL.len() {
                let what: Vec<String> = rule.what.iter().map(|k| k.name().to_owned()).collect();
                table.insert("what", value(string_array(&what)));
            }
            if !rule.networks.is_empty() {
                table.insert("networks", value(string_array(&rule.networks)));
            }
            if !rule.channels.is_empty() {
                table.insert("channels", value(string_array(&rule.channels)));
            }
            rules.push(table);
            Ok(())
        })
    }

    /// Removes every ignore rule with this mask.
    pub fn remove_ignore(&mut self, mask: &str) -> Result<Config, EditError> {
        self.edit(|doc| {
            let rules = array_of_tables_mut(doc, "ignore")?;
            let before = rules.len();
            rules.retain(|t| {
                !t.get("mask")
                    .and_then(Item::as_str)
                    .is_some_and(|m| m.eq_ignore_ascii_case(mask))
            });
            if rules.len() == before {
                return invalid(format!("no ignore rule for {mask:?}"));
            }
            Ok(())
        })
    }

    /// Adds a highlight word, or a regex if `regex` is set.
    pub fn add_highlight(&mut self, pattern: &str, regex: bool) -> Result<Config, EditError> {
        self.edit(|doc| {
            let list = highlight_list(doc, regex)?;
            if !list.iter().any(|v| v.as_str() == Some(pattern)) {
                list.push(pattern);
            }
            Ok(())
        })
    }

    /// Removes a highlight word or regex.
    pub fn remove_highlight(&mut self, pattern: &str) -> Result<Config, EditError> {
        self.edit(|doc| {
            let mut removed = false;
            for regex in [false, true] {
                let list = highlight_list(doc, regex)?;
                let before = list.len();
                list.retain(|v| v.as_str() != Some(pattern));
                removed |= list.len() != before;
            }
            if !removed {
                return invalid(format!("no highlight {pattern:?}"));
            }
            Ok(())
        })
    }
}

fn string_array(items: &[String]) -> Array {
    items.iter().map(String::as_str).collect()
}

fn table_mut<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table, EditError> {
    let item = parent.entry(key).or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(false);
        Item::Table(table)
    });
    item.as_table_mut()
        .ok_or_else(|| EditError::Invalid(format!("{key} in the config is not a table")))
}

fn array_of_tables_mut<'a>(
    doc: &'a mut DocumentMut,
    key: &str,
) -> Result<&'a mut ArrayOfTables, EditError> {
    doc.entry(key)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| EditError::Invalid(format!("{key} in the config is not a list of tables")))
}

fn networks_mut(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, EditError> {
    array_of_tables_mut(doc, "network")
}

fn highlight_list(doc: &mut DocumentMut, regex: bool) -> Result<&mut Array, EditError> {
    let key = if regex { "regex" } else { "words" };
    table_mut(doc.as_table_mut(), "highlight")?
        .entry(key)
        .or_insert_with(|| value(Array::new()))
        .as_array_mut()
        .ok_or_else(|| EditError::Invalid(format!("highlight.{key} is not a list")))
}

fn find_network(networks: &ArrayOfTables, name: &str) -> Option<usize> {
    networks.iter().position(|t| {
        t.get("name")
            .and_then(Item::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

fn apply_options(table: &mut Table, options: &NetworkOptions) {
    if let Some(autojoin) = &options.autojoin {
        table.insert("autojoin", value(string_array(autojoin)));
    }
    match options.sasl.as_deref() {
        Some("none") => {
            table.remove("sasl");
        }
        Some(mechanism) => {
            let mut sasl = Table::new();
            sasl.insert("mechanism", value(mechanism));
            if let Some(account) = &options.account {
                sasl.insert("account", value(account.as_str()));
            }
            let mut password = InlineTable::new();
            if let Some(entry) = &options.keyring {
                password.insert("keyring", entry.as_str().into());
            } else if let Some(argv) = &options.command {
                password.insert("command", Value::Array(string_array(argv)));
            }
            if !password.is_empty() {
                sasl.insert("password", value(password));
            }
            table.insert("sasl", Item::Table(sasl));
        }
        None => {}
    }
}

/// Writes `text` to a temporary file beside `path`, then renames it over
/// `path`. On Unix the file is readable only by its owner.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMENTED: &str = r#"# My Irsty config.
scrollback = 5000 # keep it short

[ui]
# I like my nick list.
nicklist = true

[[network]]
name = "libera"   # the big one
nicks = ["gawbul"]

[[network.servers]]
host = "irc.libera.chat"
"#;

    fn open(text: &str) -> (tempfile::TempDir, ConfigFile) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let (file, _) = ConfigFile::open(&path).unwrap();
        (dir, file)
    }

    /// Edits keep comments, spacing and order; only the change is new.
    #[test]
    fn edits_keep_comments_and_layout() {
        let (_dir, mut file) = open(COMMENTED);
        file.set("ui.mouse", "on").unwrap();
        file.add_server(
            "libera",
            &NewServer {
                host: "irc.eu.libera.chat".into(),
                ..NewServer::default()
            },
            &NetworkOptions::default(),
        )
        .unwrap();
        let written = std::fs::read_to_string(file.path()).unwrap();
        for kept in [
            "# My Irsty config.",
            "scrollback = 5000 # keep it short",
            "# I like my nick list.",
            "name = \"libera\"   # the big one",
        ] {
            assert!(written.contains(kept), "lost {kept:?}:\n{written}");
        }
        assert!(written.contains("mouse = true"), "{written}");
        assert!(
            written.contains("host = \"irc.eu.libera.chat\""),
            "{written}"
        );
        // Removing what we added gets back to the original text.
        file.remove("libera", Some("irc.eu.libera.chat")).unwrap();
        file.unset("ui.mouse").unwrap();
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), COMMENTED);
    }

    #[test]
    fn creates_a_config_from_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let (mut file, config) = ConfigFile::open(&path).unwrap();
        assert!(config.networks.is_empty() && !file.exists());
        let config = file
            .add_server(
                "libera",
                &NewServer {
                    host: "irc.libera.chat".into(),
                    ..NewServer::default()
                },
                &NetworkOptions {
                    nicks: Some(vec!["gawbul".into(), "gawbul_".into()]),
                    autojoin: Some(vec!["#irsty".into()]),
                    sasl: Some("scram-sha-256".into()),
                    account: Some("gawbul".into()),
                    keyring: Some("libera".into()),
                    command: None,
                },
            )
            .unwrap();
        let net = &config.networks[0];
        assert_eq!(net.nicks, ["gawbul", "gawbul_"]);
        assert_eq!(net.autojoin, ["#irsty"]);
        let sasl = net.sasl.as_ref().unwrap();
        assert_eq!(
            sasl.password,
            Some(crate::config::SecretSource::Keyring("libera".into()))
        );
        // It reads back the same.
        let (_, again) = ConfigFile::open(&path).unwrap();
        assert_eq!(again, config);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    /// A change that would make the config invalid is refused, and the file is
    /// left alone.
    #[test]
    fn invalid_changes_are_refused() {
        let (_dir, mut file) = open(COMMENTED);
        assert!(file.set("scrollback", "0").is_err());
        assert!(file.set("scrollback", "lots").is_err());
        assert!(file.set("ui.colour", "on").is_err());
        assert!(file.set("ui.mouse", "maybe").is_err());
        assert!(
            file.add_server("new", &NewServer::default(), &NetworkOptions::default())
                .is_err(),
            "a new network needs a nick"
        );
        assert!(file.remove("nope", None).is_err());
        assert!(
            file.remove("libera", Some("irc.libera.chat")).is_err(),
            "the last server can't go"
        );
        assert!(file.remove_alias("nope").is_err());
        assert!(file.remove_ignore("nobody").is_err());
        assert!(file.remove_highlight("nothing").is_err());
        let regex = file.add_highlight("(unclosed", true).unwrap_err();
        assert!(regex.to_string().contains("highlight regex"), "{regex}");
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), COMMENTED);
        assert_eq!(file.text(), COMMENTED);
    }

    #[test]
    fn aliases_ignores_highlights_and_updates() {
        let (_dir, mut file) = open(COMMENTED);
        let config = file
            .set_alias("j", &["/join $*".into()])
            .and_then(|_| file.set_alias("hi", &["/msg $channel hi".into(), "/me waves".into()]))
            .unwrap();
        assert_eq!(config.aliases.len(), 2);
        let config = file
            .add_ignore(&IgnoreRule {
                mask: "spammer".into(),
                what: vec![IgnoreKind::Messages],
                networks: vec!["libera".into()],
                channels: Vec::new(),
            })
            .unwrap();
        assert_eq!(config.ignore[0].what, [IgnoreKind::Messages]);
        let config = file
            .add_highlight("irsty", false)
            .and_then(|_| file.add_highlight("irsty", false))
            .and_then(|_| file.add_highlight(r"\bsteve\b", true))
            .unwrap();
        assert_eq!(config.highlight.words, ["irsty"], "no duplicates");
        assert_eq!(config.highlight.regex.len(), 1);
        let config = file
            .update(
                "LIBERA",
                &NetworkOptions {
                    nicks: Some(vec!["new".into()]),
                    sasl: Some("external".into()),
                    ..NetworkOptions::default()
                },
            )
            .unwrap_err();
        // External SASL needs a client certificate.
        assert!(config.to_string().contains("client_cert"), "{config}");
        let config = file
            .update(
                "libera",
                &NetworkOptions {
                    nicks: Some(vec!["new".into()]),
                    ..NetworkOptions::default()
                },
            )
            .unwrap();
        assert_eq!(config.networks[0].nicks, ["new"]);
        let config = file.bind("ctrl-g", "none").unwrap();
        assert_eq!(config.keys["ctrl-g"], "none");
        assert!(file.set("ui.keymap", "emacs").is_err());
        assert_eq!(
            file.set("ui.keymap", "weechat").unwrap().ui.keymap,
            "weechat"
        );
        let config = file.unbind("ctrl-g").unwrap();
        assert!(config.keys.is_empty());
        assert!(file.unbind("ctrl-g").is_err());
        let config = file
            .remove_alias("j")
            .and_then(|_| file.remove_ignore("SPAMMER"))
            .and_then(|_| file.remove_highlight("irsty"))
            .unwrap();
        assert_eq!(config.aliases.len(), 1);
        assert!(config.ignore.is_empty());
        assert!(config.highlight.words.is_empty());
        let config = file.remove("libera", None).unwrap();
        assert!(config.networks.is_empty());
    }
}
