use serde::{Deserialize, Serialize};
use std::{fmt, net::IpAddr};

pub const PROTOCOL_VERSION: &str = "2.1";

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Mobile,
    #[default]
    Desktop,
    Web,
    Headless,
    Server,
}

impl From<DeviceType> for localsend::model::discovery::DeviceType {
    fn from(value: DeviceType) -> Self {
        match value {
            DeviceType::Mobile => Self::Mobile,
            DeviceType::Desktop => Self::Desktop,
            DeviceType::Web => Self::Web,
            DeviceType::Headless => Self::Headless,
            DeviceType::Server => Self::Server,
        }
    }
}

impl From<localsend::model::discovery::DeviceType> for DeviceType {
    fn from(value: localsend::model::discovery::DeviceType) -> Self {
        match value {
            localsend::model::discovery::DeviceType::Mobile => Self::Mobile,
            localsend::model::discovery::DeviceType::Desktop => Self::Desktop,
            localsend::model::discovery::DeviceType::Web => Self::Web,
            localsend::model::discovery::DeviceType::Headless => Self::Headless,
            localsend::model::discovery::DeviceType::Server => Self::Server,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Http,
    Https,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<Protocol> for localsend::model::discovery::ProtocolType {
    fn from(value: Protocol) -> Self {
        match value {
            Protocol::Http => Self::Http,
            Protocol::Https => Self::Https,
        }
    }
}

impl From<localsend::model::discovery::ProtocolType> for Protocol {
    fn from(value: localsend::model::discovery::ProtocolType) -> Self {
        match value {
            localsend::model::discovery::ProtocolType::Http => Self::Http,
            localsend::model::discovery::ProtocolType::Https => Self::Https,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    pub version: String,
    pub device_model: Option<String>,
    pub device_type: Option<DeviceType>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: Protocol,
    pub download: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

impl DeviceInfo {
    pub fn to_multicast_device(&self) -> localsend::multicast::MulticastDevice {
        localsend::multicast::MulticastDevice {
            alias: self.alias.clone(),
            version: self.version.clone(),
            device_model: self.device_model.clone(),
            device_type: self.device_type.map(Into::into),
            fingerprint: self.fingerprint.clone(),
            port: self.port,
            protocol: self.protocol.into(),
            download: self.download,
        }
    }

    pub fn to_register(&self) -> localsend::http::dto_v2::RegisterDtoV2 {
        localsend::http::dto_v2::RegisterDtoV2 {
            alias: self.alias.clone(),
            version: self.version.clone(),
            device_model: self.device_model.clone(),
            device_type: self.device_type.map(Into::into),
            fingerprint: self.fingerprint.clone(),
            port: self.port,
            protocol: self.protocol.into(),
            download: self.download,
        }
    }

    pub fn from_register(value: localsend::http::dto_v2::RegisterDtoV2, ip: IpAddr) -> Self {
        Self {
            alias: value.alias,
            version: value.version,
            device_model: value.device_model,
            device_type: value.device_type.map(Into::into),
            fingerprint: value.fingerprint,
            port: value.port,
            protocol: value.protocol.into(),
            download: value.download,
            ip: Some(ip.to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileId(pub String);

impl FileId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for FileId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub id: FileId,
    pub file_name: String,
    pub size: u64,
    pub file_type: String,
    pub sha256: Option<String>,
    pub preview: Option<String>,
    pub metadata: Option<FileMetadataDetails>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadataDetails {
    pub modified: Option<String>,
    pub accessed: Option<String>,
}

impl From<FileMetadata> for localsend::model::transfer::FileDto {
    fn from(value: FileMetadata) -> Self {
        Self {
            id: value.id.0,
            file_name: value.file_name,
            size: value.size,
            file_type: value.file_type,
            sha256: value.sha256,
            preview: value.preview,
            metadata: value
                .metadata
                .map(|meta| localsend::model::transfer::FileMetadata {
                    modified: meta.modified,
                    accessed: meta.accessed,
                }),
        }
    }
}

impl From<localsend::model::transfer::FileDto> for FileMetadata {
    fn from(value: localsend::model::transfer::FileDto) -> Self {
        Self {
            id: FileId(value.id),
            file_name: value.file_name,
            size: value.size,
            file_type: value.file_type,
            sha256: value.sha256,
            preview: value.preview,
            metadata: value.metadata.map(|meta| FileMetadataDetails {
                modified: meta.modified,
                accessed: meta.accessed,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedFile {
    pub file_name: String,
    pub size: u64,
    pub sender: String,
    pub time: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnouncementMessage {
    pub alias: String,
    pub version: String,
    pub device_model: Option<String>,
    pub device_type: Option<DeviceType>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: Protocol,
    pub download: bool,
    #[serde(default)]
    pub announce: bool,
    #[serde(default)]
    pub announcement: Option<bool>,
}
