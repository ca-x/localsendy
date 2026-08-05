use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use localsend_rs::server::PendingTransfer;
use localsend_rs::{DeviceInfo, ReceivedFile};
use serde::Serialize;
use tokio::sync::{RwLock as AsyncRwLock, mpsc};
use uuid::Uuid;

use crate::config::Config;
use crate::network::{DiscoveryCommand, NetworkPreferences};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub local_device: DeviceInfo,
    pub devices: Arc<RwLock<HashMap<String, SeenDevice>>>,
    pub pending_transfer: Arc<AsyncRwLock<Option<PendingTransfer>>>,
    pub received_files: Arc<AsyncRwLock<Vec<ReceivedFile>>>,
    pub outgoing_transfers: Arc<AsyncRwLock<Vec<OutgoingTransfer>>>,
    pub scan_tx: mpsc::Sender<DiscoveryCommand>,
    pub network_preferences: Arc<RwLock<NetworkPreferences>>,
    pub started_at: Instant,
}

#[derive(Clone)]
pub struct SeenDevice {
    pub device: DeviceInfo,
    pub last_seen: Instant,
    pub source_interface: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    #[serde(flatten)]
    pub device: DeviceInfo,
    pub source_interface: Option<String>,
    pub source_interface_label: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingTransfer {
    pub id: Uuid,
    pub target_alias: String,
    pub file_names: Vec<String>,
    pub total_bytes: u64,
    pub status: TransferStatus,
    pub created_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Preparing,
    Completed,
    Failed,
}

impl AppState {
    pub fn active_devices(&self) -> Vec<DiscoveredDevice> {
        let stale_after = Duration::from_secs(120);
        let labels = self
            .network_preferences
            .read()
            .expect("network preferences lock should not be poisoned")
            .labels
            .clone();
        let mut devices = self
            .devices
            .read()
            .expect("device discovery lock should not be poisoned")
            .values()
            .filter(|seen| seen.last_seen.elapsed() <= stale_after)
            .map(|seen| DiscoveredDevice {
                device: seen.device.clone(),
                source_interface: seen.source_interface.clone(),
                source_interface_label: seen
                    .source_interface
                    .as_ref()
                    .and_then(|name| labels.get(name))
                    .cloned(),
            })
            .collect::<Vec<_>>();
        devices.sort_by_key(|device| device.device.alias.to_lowercase());
        devices
    }

    pub fn describe_device(&self, seen: &SeenDevice) -> DiscoveredDevice {
        let source_interface_label = seen.source_interface.as_ref().and_then(|name| {
            self.network_preferences
                .read()
                .expect("network preferences lock should not be poisoned")
                .labels
                .get(name)
                .cloned()
        });
        DiscoveredDevice {
            device: seen.device.clone(),
            source_interface: seen.source_interface.clone(),
            source_interface_label,
        }
    }
}
