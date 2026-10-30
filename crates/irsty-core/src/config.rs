//! The configuration model, loaded from a single `config.toml`.
//!
//! Every option is documented in `docs/CONFIG.md`. Secrets are never bare strings;
//! see [`SecretSource`] and ADR 0004.

use std::path::{Path, PathBuf};
use std::time::Duration;

use irsty_proto::Secret;
use serde::Deserialize;

/// Why the configuration could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The file is not valid TOML or does not match the schema. The config text
    /// itself is never quoted, as it may contain secrets.
    #[error("invalid configuration in {path} at line {line}, column {column}: {message}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// 1-based line of the error.
        line: usize,
        /// 1-based column of the error.
        column: usize,
        /// What is wrong.
        message: String,
    },
    /// The file parsed but makes no sense.
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// Why a secret could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The OS keyring failed or has no entry.
    #[error("keyring entry {name:?}: {source}")]
    Keyring {
        /// The entry name.
        name: String,
        /// The underlying error.
        source: keyring::Error,
    },
    /// The command could not be run, failed, or printed nothing usable.
    #[error("secret command {program:?}: {reason}")]
    Command {
        /// The program run.
        program: String,
        /// What went wrong.
        reason: String,
    },
}

/// The keyring service name under which Irsty stores secrets.
pub const KEYRING_SERVICE: &str = "irsty";

/// How long a secret command may run (it might prompt for a GPG passphrase).
const SECRET_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// Where a secret comes from. A bare string in the config is rejected.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "SecretRepr")]
pub enum SecretSource {
    /// The OS keyring, service `irsty`, entry named here.
    Keyring(String),
    /// A command whose standard output (trailing newline removed) is the secret.
    Command(Vec<String>),
    /// Plaintext in the config file. Allowed, with a warning at every start.
    Plain(Secret),
}

impl std::fmt::Debug for SecretSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keyring(name) => f.debug_tuple("Keyring").field(name).finish(),
            Self::Command(argv) => f.debug_tuple("Command").field(argv).finish(),
            Self::Plain(_) => f.write_str("Plain(<redacted>)"),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SecretRepr {
    Keyring {
        keyring: String,
    },
    Command {
        command: Vec<String>,
    },
    Plain {
        plain: String,
    },
    /// Anything else, including a bare string. The value is deliberately not kept.
    Other(serde::de::IgnoredAny),
}

impl TryFrom<SecretRepr> for SecretSource {
    type Error = String;

    fn try_from(repr: SecretRepr) -> Result<Self, Self::Error> {
        match repr {
            SecretRepr::Keyring { keyring } if !keyring.is_empty() => Ok(Self::Keyring(keyring)),
            SecretRepr::Command { command } if !command.is_empty() => Ok(Self::Command(command)),
            SecretRepr::Plain { plain } => Ok(Self::Plain(Secret::new(plain))),
            SecretRepr::Keyring { .. } | SecretRepr::Command { .. } => Err("secret source is empty".into()),
            SecretRepr::Other(_) => Err("secrets must not be bare strings; use { keyring = \"name\" }, \
                 { command = [\"pass\", \"irc/network\"] } or { plain = \"...\" } (see docs/CONFIG.md)"
                .into()),
        }
    }
}

impl SecretSource {
    /// Reports whether the secret is stored in plaintext.
    pub fn is_plain(&self) -> bool {
        matches!(self, Self::Plain(_))
    }

    /// Fetches the secret. Keyring access and commands run off the async runtime.
    pub async fn resolve(&self) -> Result<Secret, SecretError> {
        match self {
            Self::Plain(secret) => Ok(secret.clone()),
            Self::Keyring(name) => {
                let name = name.clone();
                let lookup = name.clone();
                let result = tokio::task::spawn_blocking(move || {
                    keyring::Entry::new(KEYRING_SERVICE, &lookup).and_then(|e| e.get_password())
                })
                .await;
                match result {
                    Ok(Ok(password)) => Ok(Secret::new(password)),
                    Ok(Err(source)) => Err(SecretError::Keyring { name, source }),
                    Err(join) => Err(SecretError::Keyring {
                        name,
                        source: keyring::Error::PlatformFailure(Box::new(join)),
                    }),
                }
            }
            Self::Command(argv) => run_secret_command(argv).await,
        }
    }
}

async fn run_secret_command(argv: &[String]) -> Result<Secret, SecretError> {
    let program = argv.first().cloned().unwrap_or_default();
    let fail = |reason: String| SecretError::Command {
        program: program.clone(),
        reason,
    };
    let mut command = tokio::process::Command::new(&program);
    command
        .args(argv.get(1..).unwrap_or_default())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(SECRET_COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            fail(format!(
                "timed out after {}s",
                SECRET_COMMAND_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| fail(e.to_string()))?;
    if !output.status.success() {
        // stderr may contain the secret's neighbours; report only the status.
        return Err(fail(format!("exited with {}", output.status)));
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| fail("output is not UTF-8".into()))?;
    // `pass` and friends print the secret on the first line.
    let secret = stdout.lines().next().unwrap_or_default();
    if secret.is_empty() {
        return Err(fail("printed nothing".into()));
    }
    Ok(Secret::new(secret))
}

fn default_true() -> bool {
    true
}

fn default_port() -> u16 {
    6697
}

/// One server to connect to.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Hostname or IP address.
    pub host: String,
    /// Port. Defaults to 6697.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Use TLS. Defaults to true.
    #[serde(default = "default_true")]
    pub tls: bool,
    /// Verify the server's certificate. Defaults to true. Turning this off is
    /// insecure and is flagged loudly in the UI.
    #[serde(default = "default_true")]
    pub tls_verify: bool,
    /// Server password (`PASS`), for bouncers and private servers.
    pub password: Option<SecretSource>,
}

impl std::fmt::Display for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tls = if self.tls { "+" } else { "" };
        write!(f, "{}:{tls}{}", self.host, self.port)
    }
}

/// SASL mechanism to authenticate with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaslMechanism {
    /// Account name and password.
    Plain,
    /// The TLS client certificate (CertFP).
    External,
}

/// SASL settings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaslConfig {
    /// The mechanism.
    pub mechanism: SaslMechanism,
    /// Account name; required for `plain`.
    pub account: Option<String>,
    /// Password; required for `plain`.
    pub password: Option<SecretSource>,
}

/// A TLS client certificate, for CertFP and SASL EXTERNAL.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientCertConfig {
    /// PEM certificate chain.
    pub cert: PathBuf,
    /// PEM private key. May be the same file as `cert`.
    pub key: PathBuf,
}

/// One IRC network.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// Display name, also used for log directories.
    pub name: String,
    /// Servers, tried in order with rotation on failure.
    pub servers: Vec<ServerConfig>,
    /// Nicks to try, in order.
    pub nicks: Vec<String>,
    /// Username (ident). Defaults to the first nick.
    pub username: Option<String>,
    /// Real name. Defaults to the first nick.
    pub realname: Option<String>,
    /// SASL authentication.
    pub sasl: Option<SaslConfig>,
    /// TLS client certificate.
    pub client_cert: Option<ClientCertConfig>,
    /// Channels to join once registered.
    #[serde(default)]
    pub autojoin: Vec<String>,
    /// Connect at startup. Defaults to true.
    #[serde(default = "default_true")]
    pub autoconnect: bool,
}

/// Plain-text logging.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    /// Write per-buffer logs. Defaults to true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Directory for logs. Defaults to the platform data directory.
    pub dir: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: None,
        }
    }
}

fn default_scrollback() -> usize {
    10_000
}

/// The whole configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Networks.
    #[serde(default, rename = "network")]
    pub networks: Vec<NetworkConfig>,
    /// Lines kept per buffer. Defaults to 10,000.
    #[serde(default = "default_scrollback")]
    pub scrollback: usize,
    /// Logging.
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl Config {
    /// Parses and validates configuration text.
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text).map_err(|e| {
            let offset = e.span().map_or(0, |span| span.start);
            let before = text.get(..offset).unwrap_or_default();
            ConfigError::Parse {
                path: path.to_owned(),
                line: before.matches('\n').count() + 1,
                column: before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1,
                message: e.message().to_owned(),
            }
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Reads, parses and validates a file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&text, path)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |msg: String| Err(ConfigError::Invalid(msg));
        if self.scrollback == 0 {
            return invalid("scrollback must be at least 1".into());
        }
        let mut names = std::collections::HashSet::new();
        for net in &self.networks {
            if net.name.trim().is_empty() {
                return invalid("a network has an empty name".into());
            }
            if !names.insert(net.name.to_lowercase()) {
                return invalid(format!("network {:?} is defined twice", net.name));
            }
            if net.servers.is_empty() {
                return invalid(format!("network {:?} has no servers", net.name));
            }
            if net.nicks.is_empty()
                || net
                    .nicks
                    .iter()
                    .any(|n| n.is_empty() || n.contains([' ', ',', '\r', '\n']))
            {
                return invalid(format!(
                    "network {:?} needs at least one valid nick",
                    net.name
                ));
            }
            if let Some(sasl) = &net.sasl {
                match sasl.mechanism {
                    SaslMechanism::Plain if sasl.account.is_none() || sasl.password.is_none() => {
                        return invalid(format!(
                            "network {:?}: SASL plain needs account and password",
                            net.name
                        ));
                    }
                    SaslMechanism::External if net.client_cert.is_none() => {
                        return invalid(format!(
                            "network {:?}: SASL external needs client_cert",
                            net.name
                        ));
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Lists every secret stored in plaintext, as `network: what`.
    pub fn plaintext_secrets(&self) -> Vec<String> {
        let mut found = Vec::new();
        for net in &self.networks {
            if net
                .sasl
                .as_ref()
                .and_then(|s| s.password.as_ref())
                .is_some_and(SecretSource::is_plain)
            {
                found.push(format!("{}: SASL password", net.name));
            }
            for server in &net.servers {
                if server.password.as_ref().is_some_and(SecretSource::is_plain) {
                    found.push(format!("{}: password for {server}", net.name));
                }
            }
        }
        found
    }
}

/// Platform directories for Irsty.
#[derive(Clone, Debug)]
pub struct Paths {
    /// Directory holding `config.toml`.
    pub config_dir: PathBuf,
    /// Directory for logs and other data.
    pub data_dir: PathBuf,
}

impl Paths {
    /// Platform defaults: XDG on Linux, `~/Library/Application Support` on macOS.
    pub fn platform() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", "irsty")?;
        Some(Self {
            config_dir: dirs.config_dir().to_owned(),
            data_dir: dirs.data_dir().to_owned(),
        })
    }

    /// The default config file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// The default log directory.
    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r##"
        scrollback = 500

        [[network]]
        name = "libera"
        nicks = ["gawbul", "gawbul_"]
        autojoin = ["#irsty"]

        [[network.servers]]
        host = "irc.libera.chat"

        [[network.servers]]
        host = "irc.eu.libera.chat"
        port = 6667
        tls = false

        [network.sasl]
        mechanism = "plain"
        account = "gawbul"
        password = { keyring = "libera" }

        [[network]]
        name = "bouncer"
        nicks = ["me"]

        [[network.servers]]
        host = "bnc.example"
        tls_verify = false
        password = { command = ["pass", "irc/bnc"] }
    "##;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(text, Path::new("test.toml"))
    }

    #[test]
    fn parses_example() {
        let config = parse(EXAMPLE).unwrap();
        assert_eq!(config.scrollback, 500);
        let libera = &config.networks[0];
        assert_eq!(libera.servers[0].port, 6697);
        assert!(libera.servers[0].tls && libera.servers[0].tls_verify);
        assert_eq!(libera.servers[1].to_string(), "irc.eu.libera.chat:6667");
        assert_eq!(libera.servers[0].to_string(), "irc.libera.chat:+6697");
        let sasl = libera.sasl.as_ref().unwrap();
        assert_eq!(sasl.password, Some(SecretSource::Keyring("libera".into())));
        assert!(libera.autoconnect);
        let bnc = &config.networks[1];
        assert!(!bnc.servers[0].tls_verify);
        assert_eq!(
            bnc.servers[0].password,
            Some(SecretSource::Command(vec!["pass".into(), "irc/bnc".into()]))
        );
        assert!(config.logging.enabled);
        assert!(config.plaintext_secrets().is_empty());
    }

    #[test]
    fn rejects_bare_string_secrets() {
        let text = EXAMPLE.replace(r#"{ keyring = "libera" }"#, r#""hunter2""#);
        let err = parse(&text).unwrap_err().to_string();
        assert!(err.contains("must not be bare strings"), "{err}");
        assert!(!err.contains("hunter2"), "error leaks the secret: {err}");
        assert!(err.contains("line 20, column 20"), "{err}");
    }

    #[test]
    fn plain_secrets_are_flagged_and_redacted() {
        let text = EXAMPLE.replace(r#"{ keyring = "libera" }"#, r#"{ plain = "hunter2" }"#);
        let config = parse(&text).unwrap();
        assert_eq!(config.plaintext_secrets(), ["libera: SASL password"]);
        assert!(!format!("{config:?}").contains("hunter2"));
    }

    #[test]
    fn validation_errors() {
        let cases = [
            (
                "[[network]]\nname = \"x\"\nnicks = [\"n\"]\nservers = []",
                "no servers",
            ),
            (
                "[[network]]\nname = \" \"\nnicks = [\"n\"]\n[[network.servers]]\nhost = \"h\"",
                "empty name",
            ),
            (
                "[[network]]\nname = \"x\"\nnicks = []\n[[network.servers]]\nhost = \"h\"",
                "valid nick",
            ),
            (
                "[[network]]\nname = \"x\"\nnicks = [\"a b\"]\n[[network.servers]]\nhost = \"h\"",
                "valid nick",
            ),
            (
                "[[network]]\nname = \"x\"\nnicks = [\"n\"]\n[[network.servers]]\nhost = \"h\"\n[[network]]\nname = \"X\"\nnicks = [\"n\"]\n[[network.servers]]\nhost = \"h\"",
                "defined twice",
            ),
            (
                "[[network]]\nname = \"x\"\nnicks = [\"n\"]\n[[network.servers]]\nhost = \"h\"\n[network.sasl]\nmechanism = \"plain\"",
                "account and password",
            ),
            (
                "[[network]]\nname = \"x\"\nnicks = [\"n\"]\n[[network.servers]]\nhost = \"h\"\n[network.sasl]\nmechanism = \"external\"",
                "client_cert",
            ),
            ("scrollback = 0", "scrollback"),
        ];
        for (text, expected) in cases {
            let err = parse(text).unwrap_err().to_string();
            assert!(err.contains(expected), "{text:?}: {err}");
        }
        let err = parse("bogus = 1").unwrap_err().to_string();
        assert!(err.contains("unknown field"), "{err}");
        assert!(parse("").unwrap().networks.is_empty());
    }

    #[test]
    fn load_reports_missing_files() {
        let err = Config::load(Path::new("/nonexistent/irsty.toml")).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }

    #[tokio::test]
    async fn resolves_command_and_plain_secrets() {
        let cmd = SecretSource::Command(vec!["printf".into(), "s3cret\\nsecond line\\n".into()]);
        assert_eq!(cmd.resolve().await.unwrap().expose(), "s3cret");
        let plain = SecretSource::Plain(Secret::new("p"));
        assert_eq!(plain.resolve().await.unwrap().expose(), "p");
        let failing = SecretSource::Command(vec!["false".into()]);
        assert!(
            failing
                .resolve()
                .await
                .unwrap_err()
                .to_string()
                .contains("exited")
        );
        let empty = SecretSource::Command(vec!["true".into()]);
        assert!(
            empty
                .resolve()
                .await
                .unwrap_err()
                .to_string()
                .contains("printed nothing")
        );
        let missing = SecretSource::Command(vec!["/nonexistent/irsty-secret".into()]);
        assert!(missing.resolve().await.is_err());
    }

    #[test]
    fn debug_redacts_plain() {
        let s = SecretSource::Plain(Secret::new("hunter2"));
        assert_eq!(format!("{s:?}"), "Plain(<redacted>)");
        assert!(format!("{:?}", SecretSource::Keyring("k".into())).contains('k'));
    }

    #[test]
    fn platform_paths() {
        if let Some(paths) = Paths::platform() {
            assert!(paths.config_file().ends_with("config.toml"));
            assert!(paths.log_dir().ends_with("logs"));
        }
    }
}
