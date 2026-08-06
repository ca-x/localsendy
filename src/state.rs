use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use localsendy_core::{
    DeviceIdentity, DeviceInfo, IncomingTransfer, PendingTransfer, ReceivedFile,
    localsend::http::server::ServerHandle,
};
use serde::{Serialize, Serializer};
use tokio::sync::{RwLock as AsyncRwLock, Semaphore, mpsc};
use uuid::Uuid;

use crate::config::Config;
use crate::network::{DiscoveryCommand, NetworkPreferences};
use localsendy_storage::{Database, InstanceKey, TransferRecord};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub database: Database,
    pub instance_key: InstanceKey,
    pub identity: Arc<DeviceIdentity>,
    pub local_device: Arc<RwLock<DeviceInfo>>,
    pub discovery_devices: Arc<RwLock<Vec<DeviceInfo>>>,
    pub receiver_server: Arc<ServerHandle>,
    pub auto_accept: Arc<AtomicBool>,
    pub alias_locale: Arc<RwLock<String>>,
    pub devices: Arc<RwLock<HashMap<String, SeenDevice>>>,
    pub pending_transfer: Arc<AsyncRwLock<Option<PendingTransfer>>>,
    pub received_files: Arc<AsyncRwLock<Vec<ReceivedFile>>>,
    pub incoming_transfers: Arc<AsyncRwLock<Vec<IncomingTransfer>>>,
    pub outgoing_transfers: Arc<AsyncRwLock<Vec<OutgoingTransfer>>>,
    pub send_semaphore: Arc<Semaphore>,
    pub download_root: PathBuf,
    pub download_subdirectory: Arc<AsyncRwLock<String>>,
    pub receiver_destination: Arc<AsyncRwLock<PathBuf>>,
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
    #[serde(serialize_with = "serialize_atomic_u64")]
    pub transferred_bytes: Arc<AtomicU64>,
    pub status: TransferStatus,
    pub created_at: DateTime<Utc>,
    pub error: Option<String>,
    pub content_type: Option<String>,
    pub is_clipboard: bool,
}

fn serialize_atomic_u64<S>(value: &Arc<AtomicU64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u64(value.load(Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Preparing,
    Sending,
    Completed,
    Failed,
}

pub fn restore_outgoing_transfers(records: Vec<TransferRecord>) -> Vec<OutgoingTransfer> {
    let mut positions = HashMap::<String, usize>::new();
    let mut transfers = Vec::<OutgoingTransfer>::new();
    for record in records {
        if let Some(index) = positions.get(&record.batch_id).copied() {
            let transfer = &mut transfers[index];
            transfer.file_names.push(record.file_name);
            transfer.total_bytes = transfer.total_bytes.saturating_add(record.size);
            if record.status == "completed" {
                transfer
                    .transferred_bytes
                    .fetch_add(record.size, Ordering::Relaxed);
            } else {
                transfer.status = TransferStatus::Failed;
            }
            transfer.error = transfer.error.take().or(record.error);
            transfer.is_clipboard |= record.is_clipboard;
            continue;
        }
        let Ok(id) = Uuid::parse_str(&record.batch_id) else {
            continue;
        };
        let completed = record.status == "completed";
        let created_at =
            chrono::DateTime::from_timestamp_millis(record.created_at_ms).unwrap_or_default();
        positions.insert(record.batch_id, transfers.len());
        transfers.push(OutgoingTransfer {
            id,
            target_alias: record.peer_alias,
            file_names: vec![record.file_name],
            total_bytes: record.size,
            transferred_bytes: Arc::new(AtomicU64::new(if completed { record.size } else { 0 })),
            status: if completed {
                TransferStatus::Completed
            } else {
                TransferStatus::Failed
            },
            created_at,
            error: record.error,
            content_type: record.content_type,
            is_clipboard: record.is_clipboard,
        });
    }
    transfers
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_complete_and_partial_outgoing_batches() {
        let record = |id: &str, batch_id: &str, status: &str, size: u64| TransferRecord {
            id: id.to_owned(),
            batch_id: batch_id.to_owned(),
            instance_id: "single:single".to_owned(),
            direction: "outgoing".to_owned(),
            peer_alias: "Phone".to_owned(),
            file_name: format!("{id}.bin"),
            size,
            status: status.to_owned(),
            created_at_ms: 42,
            error: (status == "failed").then(|| "declined".to_owned()),
            content_type: Some("application/octet-stream".to_owned()),
            is_clipboard: false,
        };
        let completed_id = Uuid::new_v4().to_string();
        let partial_id = Uuid::new_v4().to_string();

        let restored = restore_outgoing_transfers(vec![
            record("one", &completed_id, "completed", 2),
            record("two", &completed_id, "completed", 3),
            record("three", &partial_id, "completed", 5),
            record("four", &partial_id, "failed", 7),
        ]);

        let completed = restored
            .iter()
            .find(|transfer| transfer.id.to_string() == completed_id)
            .unwrap();
        assert!(matches!(completed.status, TransferStatus::Completed));
        assert_eq!(completed.total_bytes, 5);
        assert_eq!(completed.transferred_bytes.load(Ordering::Relaxed), 5);

        let partial = restored
            .iter()
            .find(|transfer| transfer.id.to_string() == partial_id)
            .unwrap();
        assert!(matches!(partial.status, TransferStatus::Failed));
        assert_eq!(partial.total_bytes, 12);
        assert_eq!(partial.transferred_bytes.load(Ordering::Relaxed), 5);
        assert_eq!(partial.error.as_deref(), Some("declined"));
    }
}
