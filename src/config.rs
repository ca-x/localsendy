use std::{collections::BTreeSet, env, fs, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use localsendy_core::DeviceType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::network::NetworkSelection;

#[derive(Clone, Debug)]
pub struct Config {
    pub web_bind: SocketAddr,
    pub alias: String,
    pub device_type: DeviceType,
    pub device_model: Option<String>,
    pub localsend_port: u16,
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    pub temp_dir: PathBuf,
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
        let localsend_port = parse_env("LOCALSENDY_PORT", 53317_u16)?;
        let data_dir = PathBuf::from(env_var("LOCALSENDY_DATA_DIR", "/data"));
        fs::create_dir_all(&data_dir)
            .with_context(|| format!("failed to create {}", data_dir.display()))?;
        let alias = resolve_alias(
            &data_dir,
            env::var("LOCALSENDY_ALIAS").ok(),
            env::var("LOCALSENDY_ALIAS_PREFIX").unwrap_or_default(),
            env::var("LOCALSENDY_ALIAS_LOCALE").ok(),
            env::var("LC_ALL").ok().or_else(|| env::var("LANG").ok()),
        )?;
        let device_type = parse_device_type(env::var("LOCALSENDY_DEVICE_TYPE").ok())?;
        let device_model = optional_text_env("LOCALSENDY_DEVICE_MODEL")?;
        let download_dir = env::var("LOCALSENDY_DOWNLOAD_DIR")
            .or_else(|_| env::var("LOCALSENDY_SAVE_DIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("downloads"));
        let temp_dir = env::var("LOCALSENDY_TEMP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("tmp"));
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
            device_type,
            device_model,
            localsend_port,
            data_dir,
            download_dir,
            temp_dir,
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
        self.temp_dir.clone()
    }

    pub fn network_config_path(&self) -> PathBuf {
        self.data_dir.join("network-settings.json")
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("localsendy.sqlite3")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceIdentity {
    adjective_index: usize,
    fruit_index: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyDeviceIdentity {
    generated_alias: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AliasLocale {
    English,
    SimplifiedChinese,
    TraditionalChinese,
}

const ALIAS_ADJECTIVES: &[&str] = &[
    "Adorable",
    "Beautiful",
    "Big",
    "Bright",
    "Clean",
    "Clever",
    "Cool",
    "Cute",
    "Cunning",
    "Determined",
    "Energetic",
    "Efficient",
    "Fantastic",
    "Fast",
    "Fine",
    "Fresh",
    "Good",
    "Gorgeous",
    "Great",
    "Handsome",
    "Hot",
    "Kind",
    "Lovely",
    "Mystic",
    "Neat",
    "Nice",
    "Patient",
    "Pretty",
    "Powerful",
    "Rich",
    "Secret",
    "Smart",
    "Solid",
    "Special",
    "Strategic",
    "Strong",
    "Tidy",
    "Wise",
];

const ALIAS_FRUITS: &[&str] = &[
    "Apple",
    "Avocado",
    "Banana",
    "Blackberry",
    "Blueberry",
    "Broccoli",
    "Carrot",
    "Cherry",
    "Coconut",
    "Grape",
    "Lemon",
    "Lettuce",
    "Mango",
    "Melon",
    "Mushroom",
    "Onion",
    "Orange",
    "Papaya",
    "Peach",
    "Pear",
    "Pineapple",
    "Potato",
    "Pumpkin",
    "Raspberry",
    "Strawberry",
    "Tomato",
];

const ALIAS_ADJECTIVES_ZH_CN: &[&str] = &[
    "迷人",
    "美丽",
    "巨大",
    "明亮",
    "干净",
    "聪明",
    "帅气",
    "可爱",
    "狡猾",
    "坚定",
    "有活力",
    "高效",
    "极好",
    "快速",
    "不错",
    "新鲜",
    "好",
    "华丽",
    "伟大",
    "英俊",
    "炽热",
    "善良",
    "诚实",
    "神秘",
    "整洁",
    "开心",
    "耐心",
    "漂亮",
    "强大",
    "富有",
    "秘密",
    "聪明",
    "稳固",
    "特别",
    "战略性",
    "强大",
    "整洁",
    "智慧",
];

const ALIAS_FRUITS_ZH_CN: &[&str] = &[
    "苹果",
    "鳄梨",
    "香蕉",
    "黑莓",
    "蓝莓",
    "西兰花",
    "胡萝卜",
    "樱桃",
    "椰子",
    "葡萄",
    "柠檬",
    "莴苣",
    "芒果",
    "甜瓜",
    "蘑菇",
    "洋葱",
    "橙子",
    "木瓜",
    "桃子",
    "梨",
    "菠萝",
    "土豆",
    "南瓜",
    "覆盆子",
    "草莓",
    "番茄",
];

const ALIAS_ADJECTIVES_ZH_TW: &[&str] = &[
    "迷人",
    "美麗",
    "巨大",
    "明亮",
    "乾淨",
    "聰明",
    "帥氣",
    "可愛",
    "狡猾",
    "堅定",
    "有活力",
    "高效",
    "極好",
    "快速",
    "不錯",
    "新鮮",
    "好",
    "華麗",
    "偉大",
    "英俊",
    "熾熱",
    "善良",
    "誠實",
    "神秘",
    "整潔",
    "開心",
    "耐心",
    "漂亮",
    "強大",
    "富有",
    "秘密",
    "聰明",
    "穩固",
    "特別",
    "戰略性",
    "強大",
    "整潔",
    "智慧",
];

const ALIAS_FRUITS_ZH_TW: &[&str] = &[
    "蘋果",
    "酪梨",
    "香蕉",
    "黑莓",
    "藍莓",
    "花椰菜",
    "胡蘿蔔",
    "櫻桃",
    "椰子",
    "葡萄",
    "檸檬",
    "萵苣",
    "芒果",
    "甜瓜",
    "蘑菇",
    "洋蔥",
    "柳橙",
    "木瓜",
    "桃子",
    "梨",
    "鳳梨",
    "馬鈴薯",
    "南瓜",
    "覆盆子",
    "草莓",
    "番茄",
];

fn resolve_alias(
    data_dir: &std::path::Path,
    alias: Option<String>,
    prefix: String,
    locale: Option<String>,
    system_locale: Option<String>,
) -> Result<String> {
    if let Some(alias) = alias {
        return validate_text("LOCALSENDY_ALIAS", alias, 64, false);
    }

    let prefix = validate_text("LOCALSENDY_ALIAS_PREFIX", prefix, 32, true)?;
    let locale = parse_alias_locale(locale, system_locale)?;
    let path = data_dir.join("device-identity.json");
    let identity = match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<DeviceIdentity>(&bytes) {
            Ok(identity) => identity,
            Err(indexed_error) => {
                let legacy =
                    serde_json::from_slice::<LegacyDeviceIdentity>(&bytes).with_context(|| {
                        format!(
                            "failed to parse {} as indexed identity ({indexed_error})",
                            path.display()
                        )
                    })?;
                let identity = identity_from_legacy_alias(&legacy.generated_alias)
                    .with_context(|| format!("failed to migrate {}", path.display()))?;
                write_identity(&path, &identity)?;
                identity
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let identity = generate_random_identity();
            write_identity(&path, &identity)?;
            identity
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let generated_alias = localized_alias(&identity, locale)?;

    let alias = if prefix.is_empty() {
        generated_alias
    } else {
        format!("{prefix} {generated_alias}")
    };
    validate_text("generated device alias", alias, 96, false)
}

fn generate_random_identity() -> DeviceIdentity {
    let seed = Uuid::new_v4().as_u128();
    DeviceIdentity {
        adjective_index: (seed as usize) % ALIAS_ADJECTIVES.len(),
        fruit_index: ((seed >> 64) as usize) % ALIAS_FRUITS.len(),
    }
}

fn write_identity(path: &std::path::Path, identity: &DeviceIdentity) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(identity)?)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("failed to replace {}", path.display()))
}

fn identity_from_legacy_alias(alias: &str) -> Result<DeviceIdentity> {
    for (adjective_index, adjective) in ALIAS_ADJECTIVES.iter().enumerate() {
        for (fruit_index, fruit) in ALIAS_FRUITS.iter().enumerate() {
            if alias == format!("{adjective} {fruit}") {
                return Ok(DeviceIdentity {
                    adjective_index,
                    fruit_index,
                });
            }
        }
    }
    bail!("legacy generated alias is not an official LocalSend word pair")
}

fn localized_alias(identity: &DeviceIdentity, locale: AliasLocale) -> Result<String> {
    let (adjectives, fruits, separator): (&[&str], &[&str], &str) = match locale {
        AliasLocale::English => (ALIAS_ADJECTIVES, ALIAS_FRUITS, " "),
        AliasLocale::SimplifiedChinese => (ALIAS_ADJECTIVES_ZH_CN, ALIAS_FRUITS_ZH_CN, "的"),
        AliasLocale::TraditionalChinese => (ALIAS_ADJECTIVES_ZH_TW, ALIAS_FRUITS_ZH_TW, "的"),
    };
    let adjective = adjectives
        .get(identity.adjective_index)
        .context("device identity adjective index is out of range")?;
    let fruit = fruits
        .get(identity.fruit_index)
        .context("device identity fruit index is out of range")?;
    Ok(format!("{adjective}{separator}{fruit}"))
}

fn parse_alias_locale(value: Option<String>, system_locale: Option<String>) -> Result<AliasLocale> {
    let requested = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("auto");
    if requested.eq_ignore_ascii_case("auto") {
        return Ok(locale_from_tag(system_locale.as_deref().unwrap_or("en")));
    }
    let locale = locale_from_tag(requested);
    if matches!(
        normalize_locale_tag(requested).as_str(),
        "en" | "zh-cn" | "zh-hans" | "zh-tw" | "zh-hk" | "zh-mo" | "zh-hant"
    ) {
        Ok(locale)
    } else {
        bail!("LOCALSENDY_ALIAS_LOCALE must be auto, en, zh-CN, or zh-TW")
    }
}

fn locale_from_tag(value: &str) -> AliasLocale {
    match normalize_locale_tag(value).as_str() {
        tag if tag == "zh-cn" || tag == "zh-hans" || tag.starts_with("zh-hans-") => {
            AliasLocale::SimplifiedChinese
        }
        tag if tag == "zh-tw"
            || tag == "zh-hk"
            || tag == "zh-mo"
            || tag == "zh-hant"
            || tag.starts_with("zh-hant-") =>
        {
            AliasLocale::TraditionalChinese
        }
        tag if tag.starts_with("zh-") || tag == "zh" => AliasLocale::SimplifiedChinese,
        _ => AliasLocale::English,
    }
}

fn normalize_locale_tag(value: &str) -> String {
    value
        .split(['.', '@'])
        .next()
        .unwrap_or(value)
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase()
}

fn parse_device_type(value: Option<String>) -> Result<DeviceType> {
    match value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("server")
        .to_ascii_lowercase()
        .as_str()
    {
        "mobile" => Ok(DeviceType::Mobile),
        "desktop" => Ok(DeviceType::Desktop),
        "web" => Ok(DeviceType::Web),
        "headless" => Ok(DeviceType::Headless),
        "server" => Ok(DeviceType::Server),
        _ => bail!("LOCALSENDY_DEVICE_TYPE must be mobile, desktop, web, headless, or server"),
    }
}

fn optional_text_env(key: &str) -> Result<Option<String>> {
    env::var(key)
        .ok()
        .map(|value| validate_text(key, value, 64, false))
        .transpose()
}

fn validate_text(key: &str, value: String, max_chars: usize, allow_empty: bool) -> Result<String> {
    let value = value.trim().to_owned();
    if !allow_empty && value.is_empty() {
        bail!("{key} cannot be empty");
    }
    if value.chars().any(char::is_control) {
        bail!("{key} cannot contain control characters");
    }
    if value.chars().count() > max_chars {
        bail!("{key} must be {max_chars} characters or fewer");
    }
    Ok(value)
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
    use std::fs;

    use localsendy_core::DeviceType;
    use uuid::Uuid;

    use super::{
        AliasLocale, DeviceIdentity, localized_alias, parse_alias_locale, parse_bool_env,
        parse_device_type, parse_network_selection, resolve_alias,
    };
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

    #[test]
    fn locale_parser_supports_explicit_and_system_locales() {
        assert_eq!(
            parse_alias_locale(Some("zh-CN".to_owned()), None).unwrap(),
            AliasLocale::SimplifiedChinese
        );
        assert_eq!(
            parse_alias_locale(Some("auto".to_owned()), Some("zh_TW.UTF-8".to_owned())).unwrap(),
            AliasLocale::TraditionalChinese
        );
        assert_eq!(
            parse_alias_locale(None, Some("en_US.UTF-8".to_owned())).unwrap(),
            AliasLocale::English
        );
        assert!(parse_alias_locale(Some("fr".to_owned()), None).is_err());
    }

    #[test]
    fn localized_alias_uses_the_same_identity_indexes() {
        let identity = DeviceIdentity {
            adjective_index: 7,
            fruit_index: 1,
        };
        assert_eq!(
            localized_alias(&identity, AliasLocale::English).unwrap(),
            "Cute Avocado"
        );
        assert_eq!(
            localized_alias(&identity, AliasLocale::SimplifiedChinese).unwrap(),
            "可爱的鳄梨"
        );
        assert_eq!(
            localized_alias(&identity, AliasLocale::TraditionalChinese).unwrap(),
            "可愛的酪梨"
        );
    }

    #[test]
    fn generated_identity_is_stable_across_locale_and_prefix_changes() {
        let directory = std::env::temp_dir().join(format!("localsendy-config-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();

        let english =
            resolve_alias(&directory, None, String::new(), Some("en".to_owned()), None).unwrap();
        let chinese = resolve_alias(
            &directory,
            None,
            "Home".to_owned(),
            Some("zh-CN".to_owned()),
            None,
        )
        .unwrap();

        assert!(!english.is_empty());
        assert!(chinese.starts_with("Home "));
        let identity: DeviceIdentity =
            serde_json::from_slice(&fs::read(directory.join("device-identity.json")).unwrap())
                .unwrap();
        assert_eq!(
            english,
            localized_alias(&identity, AliasLocale::English).unwrap()
        );
        assert_eq!(
            chinese,
            format!(
                "Home {}",
                localized_alias(&identity, AliasLocale::SimplifiedChinese).unwrap()
            )
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_alias_overrides_generation_and_prefix() {
        let directory = std::env::temp_dir().join(format!("localsendy-config-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let alias = resolve_alias(
            &directory,
            Some("Fixed Node".to_owned()),
            "Ignored".to_owned(),
            Some("zh-CN".to_owned()),
            None,
        )
        .unwrap();
        assert_eq!(alias, "Fixed Node");
        assert!(!directory.join("device-identity.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn device_type_parser_accepts_standard_protocol_values() {
        assert!(matches!(
            parse_device_type(Some("desktop".to_owned())).unwrap(),
            DeviceType::Desktop
        ));
        assert!(matches!(
            parse_device_type(None).unwrap(),
            DeviceType::Server
        ));
        assert!(parse_device_type(Some("custom".to_owned())).is_err());
    }
}
