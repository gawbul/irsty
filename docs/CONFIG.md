# Configuration

Irsty reads a single `config.toml` from the platform config directory (`$XDG_CONFIG_HOME/irsty/` on Linux, `~/Library/Application Support/irsty/` on macOS).

Every option will be documented here, with examples, as it lands. Configuration arrives in milestone M2.

Secrets never appear as bare strings; see [ADR 0004](adr/0004-secrets.md).
