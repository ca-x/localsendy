use std::{collections::BTreeSet, env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};

use crate::network::NetworkSelection;

#[derive(Clone, Debug)]
pub struct Config {
    pub web_bind: SocketAddr,
    pub alias: String,
    pub localsend_port: u16,
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    pub auto_accept: bool,
    pub discovery_interval_seconds: u64,
    pub max_upload_bytes: u64,
    pub network_selection: NetworkSelection,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let web_bind = env_var("LOCALSENDY_BIND", "0.0.0.0:8080")
            .parse()
            .context("LOCALSENDY_BIND must be a socket address such as 0.0.0.0:8080")?;
        let alias = env_var("LOCALSENDY_ALIAS", "Localsendy");
        let localsend_port = parse_env("LOCALSENDY_PORT", 53317_u16)?;
        let data_dir = PathBuf::from(env_var("LOCALSENDY_DATA_DIR", "/data"));
        let download_dir = env::var("LOCALSENDY_DOWNLOAD_DIR")
            .or_else(|_| env::var("LOCALSENDY_SAVE_DIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("downloads"));
        let auto_accept = parse_bool_env("LOCALSENDY_AUTO_ACCEPT", false)?;
        let discovery_interval_seconds =
            parse_env("LOCALSENDY_DISCOVERY_INTERVAL_SECONDS", 30_u64)?;
        let max_upload_bytes = parse_env("LOCALSENDY_MAX_UPLOAD_BYTES", 10_737_418_240_u64)?;
        let network_selection =
            parse_network_selection(env::var("LOCALSENDY_NETWORK_INTERFACES").ok())?;

        if alias.trim().is_empty() {
            bail!("LOCALSENDY_ALIAS cannot be empty");
        }
        if discovery_interval_seconds < 5 {
            bail!("LOCALSENDY_DISCOVERY_INTERVAL_SECONDS must be at least 5");
        }
        if max_upload_bytes == 0 {
            bail!("LOCALSENDY_MAX_UPLOAD_BYTES must be greater than zero");
        }

        Ok(Self {
            web_bind,
            alias,
            localsend_port,
            data_dir,
            download_dir,
            auto_accept,
            discovery_interval_seconds,
            max_upload_bytes,
            network_selection,
        })
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.download_dir.clone()
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }

    pub fn network_config_path(&self) -> PathBuf {
        self.data_dir.join("network-settings.json")
    }
}

fn parse_network_selection(value: Option<String>) -> Result<NetworkSelection> {
    let Some(value) = value else {
        return Ok(NetworkSelection::all());
    };
    let value = value.trim();
    if value.is_empty() || value == "*" || value.eq_ignore_ascii_case("all") {
        return Ok(NetworkSelection::all());
    }

    let interfaces = value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    if interfaces.is_empty() {
        bail!(
            "LOCALSENDY_NETWORK_INTERFACES must be 'all', '*', or a comma-separated interface list"
        );
    }
    Ok(NetworkSelection::selected(interfaces))
}

fn env_var(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn parse_env<T>(key: &str, default: T) -> Result<T>
where
    T: std::str::FromStr + ToString,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    env_var(key, &default.to_string())
        .parse()
        .with_context(|| format!("{key} has an invalid value"))
}

fn parse_bool_env(key: &str, default: bool) -> Result<bool> {
    match env_var(key, if default { "true" } else { "false" })
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("{key} must be true or false"),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_bool_env, parse_network_selection};
    use crate::network::NetworkMode;

    #[test]
    fn boolean_parser_uses_default_when_missing() {
        assert!(!parse_bool_env("LOCALSENDY_TEST_MISSING_BOOL", false).unwrap());
    }

    #[test]
    fn network_selection_accepts_all_and_named_interfaces() {
        assert_eq!(
            parse_network_selection(None).unwrap().mode,
            NetworkMode::All
        );
        let selected = parse_network_selection(Some("enp2s0, wlp129s0".to_owned())).unwrap();
        assert_eq!(selected.mode, NetworkMode::Selected);
        assert!(selected.interfaces.contains("enp2s0"));
        assert!(selected.interfaces.contains("wlp129s0"));
    }
}
