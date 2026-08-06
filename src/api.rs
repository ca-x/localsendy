use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State, multipart::Field},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bytes::Bytes;
use chrono::Utc;
use futures_util::{StreamExt, stream};
use localsend::{
    http::{client::v2::LsHttpClientV2, dto_v2::PrepareUploadRequestDtoV2},
    model::transfer::{FileContent, FileDto},
};
use localsendy_core::{DeviceInfo, DeviceType, FileId, Protocol, ReceivedFile};
use localsendy_storage::SettingScope;
use localsendy_storage::TransferRecord;
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncWriteExt};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use crate::config::{normalize_alias_locale, resolve_alias, validate_text};
use crate::network::{
    DEFAULT_MULTICAST_ADDRESS, DEFAULT_MULTICAST_GROUP_V6, DiscoveryCommand, NetworkMode,
    NetworkPreferences, NetworkSelection, NetworkSettings, network_settings,
    route_interface_for_ip, save_preferences,
};
use crate::state::{AppState, DiscoveredDevice, OutgoingTransfer, SeenDevice, TransferStatus};

const MAX_SEND_TARGETS: usize = 32;
const MAX_SEND_FILES: usize = 100;
pub(crate) const MAX_CONCURRENT_SENDS: usize = 4;
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
const MAX_TEXT_REQUEST_BYTES: usize = MAX_TEXT_BYTES as usize * 6 + 64 * 1024;
const MAX_TARGET_FIELD_BYTES: usize = 256 * 1024;
const MAX_PIN_FIELD_BYTES: usize = 128;
const MAX_IN_MEMORY_TRANSFERS: usize = 100;
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);
const TEMP_UPLOAD_PREFIX: &str = "send-";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/settings", get(settings).put(update_settings))
        .route("/devices", get(devices))
        .route("/devices/scan", post(scan))
        .route("/devices/probe", post(probe_device))
        .route("/networks", get(networks).put(update_networks))
        .route("/storage", get(storage).put(update_storage))
        .route(
            "/storage/directories",
            get(storage_directories).post(create_storage_directory),
        )
        .route("/pending", get(pending))
        .route("/pending/{decision}", post(decide_pending))
        .route("/history", get(history))
        .route("/transfers", get(transfers))
        .route("/transfers/incoming", get(incoming_transfers))
        .route("/send", post(send_files).layer(DefaultBodyLimit::disable()))
        .route(
            "/send/text",
            post(send_text).layer(DefaultBodyLimit::max(MAX_TEXT_REQUEST_BYTES)),
        )
        .merge(crate::link_share::control_router())
        .fallback(api_not_found)
        .with_state(state)
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("API endpoint not found")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    version: &'static str,
    alias: String,
    device_type: Option<DeviceType>,
    device_model: Option<String>,
    web_address: String,
    localsend_port: u16,
    protocol: String,
    multicast_ipv4: String,
    multicast_ipv6: String,
    data_directory: String,
    auto_accept: bool,
    discovery_interval_seconds: u64,
    max_upload_bytes: u64,
    uptime_seconds: u64,
    nearby_devices: usize,
}

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let devices = state.active_devices();
    let local_device = state
        .local_device
        .read()
        .expect("local device lock should not be poisoned")
        .clone();
    let data_directory = state
        .receiver_destination
        .read()
        .await
        .display()
        .to_string();
    Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        alias: local_device.alias,
        device_type: local_device.device_type,
        device_model: local_device.device_model,
        web_address: state.config.web_bind.to_string(),
        localsend_port: state.config.localsend_port,
        protocol: local_device.protocol.to_string(),
        multicast_ipv4: DEFAULT_MULTICAST_ADDRESS.to_owned(),
        multicast_ipv6: DEFAULT_MULTICAST_GROUP_V6.to_string(),
        data_directory,
        auto_accept: state.auto_accept.load(Ordering::Relaxed),
        discovery_interval_seconds: state.config.discovery_interval_seconds,
        max_upload_bytes: state.config.max_upload_bytes,
        uptime_seconds: state.started_at.elapsed().as_secs(),
        nearby_devices: devices.len(),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentSettingsResponse {
    auto_accept: bool,
    alias: String,
    alias_locale: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSettingsRequest {
    auto_accept: Option<bool>,
    alias: Option<String>,
    alias_locale: Option<String>,
}

async fn settings(State(state): State<AppState>) -> Json<EnvironmentSettingsResponse> {
    let alias = state
        .local_device
        .read()
        .expect("local device lock should not be poisoned")
        .alias
        .clone();
    let alias_locale = state
        .alias_locale
        .read()
        .expect("alias locale lock should not be poisoned")
        .clone();
    Json(EnvironmentSettingsResponse {
        auto_accept: state.auto_accept.load(Ordering::Relaxed),
        alias,
        alias_locale,
    })
}

async fn update_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateSettingsRequest>,
) -> Result<Json<EnvironmentSettingsResponse>, ApiError> {
    let current_alias = state
        .local_device
        .read()
        .expect("local device lock should not be poisoned")
        .alias
        .clone();
    let current_locale = state
        .alias_locale
        .read()
        .expect("alias locale lock should not be poisoned")
        .clone();

    let requested_locale = request
        .alias_locale
        .as_deref()
        .map(str::trim)
        .filter(|locale| !locale.is_empty())
        .unwrap_or(current_locale.as_str())
        .to_owned();
    let alias_locale = normalize_alias_locale(&requested_locale).map_err(ApiError::bad_request)?;
    let alias = match request.alias {
        Some(value) if value.trim().is_empty() => resolve_alias(
            &state.config.data_dir,
            None,
            String::new(),
            Some(alias_locale.clone()),
            std::env::var("LC_ALL")
                .ok()
                .or_else(|| std::env::var("LANG").ok()),
        )
        .map_err(ApiError::bad_request)?,
        Some(value) => {
            validate_text("LOCALSENDY_ALIAS", value, 64, false).map_err(ApiError::bad_request)?
        }
        None => current_alias,
    };

    if let Some(auto_accept) = request.auto_accept {
        state
            .database
            .store_setting(
                SettingScope::Instance(&state.instance_key),
                "auto_accept",
                &auto_accept,
            )
            .map_err(ApiError::internal)?;
        state.auto_accept.store(auto_accept, Ordering::Relaxed);
    }

    let alias_changed = alias
        != state
            .local_device
            .read()
            .expect("local device lock should not be poisoned")
            .alias;
    if alias_changed {
        state
            .database
            .update_instance_alias(&state.instance_key, &alias)
            .map_err(ApiError::internal)?;
        {
            let mut local = state
                .local_device
                .write()
                .expect("local device lock should not be poisoned");
            local.alias = alias.clone();
        }
        {
            let mut devices = state
                .discovery_devices
                .write()
                .expect("discovery devices lock should not be poisoned");
            for device in devices.iter_mut() {
                if device.fingerprint
                    == state
                        .local_device
                        .read()
                        .expect("local device lock should not be poisoned")
                        .fingerprint
                {
                    device.alias = alias.clone();
                }
            }
        }
        state.receiver_server.update_alias(alias.clone()).await;
        // Do not make peers wait for the periodic announcement after a user
        // changes the display name. The discovery task snapshots this shared
        // device record when it handles the command.
        queue_discovery_scan(&state.scan_tx)?;
    }

    if request.alias_locale.is_some() || alias_changed {
        state
            .database
            .store_setting(
                SettingScope::Instance(&state.instance_key),
                "alias_locale",
                &alias_locale,
            )
            .map_err(ApiError::internal)?;
        *state
            .alias_locale
            .write()
            .expect("alias locale lock should not be poisoned") = alias_locale.clone();
    }

    Ok(Json(EnvironmentSettingsResponse {
        auto_accept: state.auto_accept.load(Ordering::Relaxed),
        alias,
        alias_locale,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageResponse {
    root: String,
    subdirectory: String,
    resolved_path: String,
}

async fn storage(State(state): State<AppState>) -> Json<StorageResponse> {
    storage_response(&state).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStorageRequest {
    subdirectory: String,
}

async fn update_storage(
    State(state): State<AppState>,
    Json(request): Json<UpdateStorageRequest>,
) -> Result<Json<StorageResponse>, ApiError> {
    let (subdirectory, destination) =
        crate::storage_path::resolve_subdirectory(&state.download_root, &request.subdirectory)
            .await
            .map_err(ApiError::bad_request)?;
    state
        .database
        .store_setting(
            SettingScope::Instance(&state.instance_key),
            "save_subdirectory",
            &subdirectory,
        )
        .map_err(ApiError::internal)?;
    *state.download_subdirectory.write().await = subdirectory;
    *state.receiver_destination.write().await = destination;
    Ok(storage_response(&state).await)
}

async fn storage_response(state: &AppState) -> Json<StorageResponse> {
    Json(StorageResponse {
        root: state.download_root.display().to_string(),
        subdirectory: state.download_subdirectory.read().await.clone(),
        resolved_path: state
            .receiver_destination
            .read()
            .await
            .display()
            .to_string(),
    })
}

#[derive(Default, Deserialize)]
struct DirectoryQuery {
    path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryListingResponse {
    path: String,
    parent: Option<String>,
    directories: Vec<String>,
}

async fn storage_directories(
    State(state): State<AppState>,
    Query(query): Query<DirectoryQuery>,
) -> Result<Json<DirectoryListingResponse>, ApiError> {
    let (path, directories) = crate::storage_path::list_subdirectories(
        &state.download_root,
        query.path.as_deref().unwrap_or_default(),
    )
    .await
    .map_err(ApiError::bad_request)?;
    let parent = path
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .or_else(|| (!path.is_empty()).then(String::new));
    Ok(Json(DirectoryListingResponse {
        path,
        parent,
        directories,
    }))
}

#[derive(Deserialize)]
struct CreateDirectoryRequest {
    parent: String,
    name: String,
}

async fn create_storage_directory(
    State(state): State<AppState>,
    Json(request): Json<CreateDirectoryRequest>,
) -> Result<Json<DirectoryListingResponse>, ApiError> {
    crate::storage_path::create_subdirectory(&state.download_root, &request.parent, &request.name)
        .await
        .map_err(ApiError::bad_request)?;
    storage_directories(
        State(state),
        Query(DirectoryQuery {
            path: Some(request.parent),
        }),
    )
    .await
}

async fn devices(State(state): State<AppState>) -> Json<Vec<DiscoveredDevice>> {
    Json(state.active_devices())
}

async fn scan(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    queue_discovery_scan(&state.scan_tx)?;
    Ok(StatusCode::ACCEPTED)
}

fn queue_discovery_scan(
    scan_tx: &tokio::sync::mpsc::Sender<DiscoveryCommand>,
) -> Result<(), ApiError> {
    match scan_tx.try_send(DiscoveryCommand::Announce) {
        Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => Ok(()),
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            Err(ApiError::internal("Discovery service is not running"))
        }
    }
}

async fn networks(State(state): State<AppState>) -> Result<Json<NetworkSettings>, ApiError> {
    let preferences = state
        .network_preferences
        .read()
        .expect("network preferences lock should not be poisoned")
        .clone();
    Ok(Json(
        network_settings(&preferences).map_err(ApiError::internal)?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateNetworksRequest {
    mode: NetworkMode,
    #[serde(default)]
    selected_interfaces: Vec<String>,
    #[serde(default)]
    labels: HashMap<String, String>,
}

async fn update_networks(
    State(state): State<AppState>,
    Json(request): Json<UpdateNetworksRequest>,
) -> Result<Json<NetworkSettings>, ApiError> {
    let available = network_settings(&NetworkPreferences::new(NetworkSelection::all()))
        .map_err(ApiError::internal)?;
    let selectable = available
        .interfaces
        .iter()
        .filter(|interface| interface.discovery_capable)
        .map(|interface| interface.name.as_str())
        .collect::<std::collections::HashSet<_>>();

    let selected_interfaces = request
        .selected_interfaces
        .into_iter()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    if request.mode == NetworkMode::Selected {
        if selected_interfaces.is_empty() {
            return Err(ApiError::bad_request(
                "Select at least one multicast-capable network interface",
            ));
        }
        if let Some(name) = selected_interfaces
            .iter()
            .find(|name| !selectable.contains(name.as_str()))
        {
            return Err(ApiError::bad_request(format!(
                "Network interface '{name}' is unavailable or does not support IPv4 or IPv6 multicast"
            )));
        }
    }

    let available_names = available
        .interfaces
        .iter()
        .map(|interface| interface.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut labels = std::collections::BTreeMap::new();
    for (name, label) in request.labels {
        if !available_names.contains(name.as_str()) {
            continue;
        }
        let label = label.trim();
        if label.chars().any(char::is_control) {
            return Err(ApiError::bad_request(
                "Interface labels cannot contain control characters",
            ));
        }
        if label.chars().count() > 64 {
            return Err(ApiError::bad_request(
                "Interface labels must be 64 characters or fewer",
            ));
        }
        if !label.is_empty() {
            labels.insert(name, label.to_owned());
        }
    }

    let selection = match request.mode {
        NetworkMode::All => NetworkSelection::all(),
        NetworkMode::Selected => NetworkSelection::selected(selected_interfaces),
    };
    let preferences = NetworkPreferences { selection, labels };
    save_preferences(&state.config.network_config_path(), &preferences)
        .await
        .map_err(ApiError::internal)?;
    *state
        .network_preferences
        .write()
        .expect("network preferences lock should not be poisoned") = preferences.clone();
    state
        .scan_tx
        .send(DiscoveryCommand::Reconfigure)
        .await
        .map_err(|_| ApiError::internal("Discovery service is not running"))?;

    Ok(Json(
        network_settings(&preferences).map_err(ApiError::internal)?,
    ))
}

#[derive(Deserialize)]
struct ProbeRequest {
    address: String,
}

async fn probe_device(
    State(state): State<AppState>,
    Json(request): Json<ProbeRequest>,
) -> Result<Json<DiscoveredDevice>, ApiError> {
    let (ip, port) = parse_probe_address(&request.address)?;
    let host = ip.to_string();
    let client = LsHttpClientV2::try_new(
        &state.identity.material.private_key_pem,
        &state.identity.material.certificate_pem,
        None,
        Some(Duration::from_secs(4)),
    )
    .map_err(ApiError::internal)?;
    let mut last_error = None;

    for protocol in [Protocol::Https, Protocol::Http] {
        match client.info(protocol.into(), &host, port).await {
            Ok(info) => {
                let device = DeviceInfo {
                    alias: info.alias,
                    version: info.version,
                    device_model: info.device_model,
                    device_type: info.device_type.map(Into::into),
                    fingerprint: info.fingerprint,
                    port,
                    protocol,
                    download: info.download,
                    ip: Some(host.clone()),
                };
                let seen = SeenDevice {
                    device: device.clone(),
                    last_seen: Instant::now(),
                    source_interface: route_interface_for_ip(ip).unwrap_or(None),
                };
                let response = state.describe_device(&seen);
                state
                    .devices
                    .write()
                    .expect("device discovery lock should not be poisoned")
                    .insert(device.fingerprint.clone(), seen);
                return Ok(Json(response));
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }
    }

    Err(ApiError::bad_gateway(last_error.unwrap_or_else(|| {
        format!("No LocalSend device responded at {ip}:{port}")
    })))
}

fn parse_probe_address(value: &str) -> Result<(IpAddr, u16), ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApiError::bad_request("Enter an IP address"));
    }
    if let Ok(address) = value.parse::<SocketAddr>() {
        return Ok((address.ip(), address.port()));
    }
    let ip = value
        .parse::<IpAddr>()
        .map_err(|_| ApiError::bad_request("Use an IP address such as 192.168.1.50"))?;
    Ok((ip, 53317))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingResponse {
    sender: DeviceInfo,
    files: Vec<PendingFile>,
    total_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingFile {
    id: String,
    name: String,
    size: u64,
    file_type: String,
}

async fn pending(State(state): State<AppState>) -> Json<Option<PendingResponse>> {
    let pending = state.pending_transfer.read().await;
    Json(pending.as_ref().map(|transfer| {
        let mut files = transfer
            .files
            .values()
            .map(|file| PendingFile {
                id: file.id.to_string(),
                name: file.file_name.clone(),
                size: file.size,
                file_type: file.file_type.clone(),
            })
            .collect::<Vec<_>>();
        files.sort_by(|a, b| a.name.cmp(&b.name));
        PendingResponse {
            sender: transfer.sender.clone(),
            total_bytes: files.iter().map(|file| file.size).sum(),
            files,
        }
    }))
}

async fn decide_pending(
    State(state): State<AppState>,
    AxumPath(decision): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let accepted = match decision.as_str() {
        "accept" => true,
        "reject" => false,
        _ => return Err(ApiError::bad_request("Decision must be accept or reject")),
    };

    let transfer = state
        .pending_transfer
        .write()
        .await
        .take()
        .ok_or_else(|| ApiError::not_found("There is no pending transfer"))?;
    transfer
        .response_tx
        .send(accepted)
        .map_err(|_| ApiError::conflict("The pending transfer has already expired"))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn history(State(state): State<AppState>) -> Json<Vec<ReceivedFile>> {
    let mut files = state.received_files.read().await.clone();
    files.reverse();
    Json(files)
}

async fn transfers(State(state): State<AppState>) -> Json<Vec<OutgoingTransfer>> {
    let mut current = state.outgoing_transfers.read().await.clone();
    current.sort_by_key(|transfer| Reverse(transfer.created_at));
    current.truncate(100);
    Json(current)
}

async fn incoming_transfers(
    State(state): State<AppState>,
) -> Json<Vec<localsendy_core::IncomingTransfer>> {
    let mut transfers = state.incoming_transfers.read().await.clone();
    transfers.reverse();
    Json(transfers)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendResponse {
    transfer_id: Uuid,
    transfers: Vec<SendTargetResponse>,
    files_sent: usize,
    total_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendTargetResponse {
    transfer_id: Uuid,
    target_alias: String,
    files_sent: usize,
    total_bytes: u64,
    success: bool,
    error: Option<String>,
}

struct SendOutcome {
    accepted_ids: HashSet<String>,
    total_bytes: u64,
}

struct SendFailure {
    error: ApiError,
    completed_ids: HashSet<String>,
    completed_bytes: u64,
}

impl From<ApiError> for SendFailure {
    fn from(error: ApiError) -> Self {
        Self {
            error,
            completed_ids: HashSet::new(),
            completed_bytes: 0,
        }
    }
}

struct RemoteSessionGuard {
    signal: Option<tokio::sync::oneshot::Sender<bool>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl RemoteSessionGuard {
    fn arm(
        state: &AppState,
        protocol: localsend::model::discovery::ProtocolType,
        host: &str,
        port: u16,
        session_id: &str,
        expected_fingerprint: Option<String>,
    ) -> Self {
        let private_key = state.identity.material.private_key_pem.clone();
        let certificate = state.identity.material.certificate_pem.clone();
        let host = host.to_owned();
        let session_id = session_id.to_owned();
        let (signal, receiver) = tokio::sync::oneshot::channel::<bool>();
        let task = tokio::spawn(async move {
            if matches!(receiver.await, Ok(true)) {
                return;
            }
            let client = match LsHttpClientV2::try_new(
                &private_key,
                &certificate,
                expected_fingerprint,
                None,
            ) {
                Ok(client) => client,
                Err(error) => {
                    warn!(%error, %session_id, "failed to create LocalSend cancellation client");
                    return;
                }
            };
            match tokio::time::timeout(
                CANCEL_TIMEOUT,
                client.cancel(protocol, &host, port, &session_id),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    warn!(%error, %session_id, "failed to cancel remote LocalSend session")
                }
                Err(_) => warn!(%session_id, "timed out cancelling remote LocalSend session"),
            }
        });
        Self {
            signal: Some(signal),
            task: Some(task),
        }
    }

    async fn complete(mut self) {
        if let Some(signal) = self.signal.take() {
            let _ = signal.send(true);
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    async fn cancel(mut self) {
        if let Some(signal) = self.signal.take() {
            let _ = signal.send(false);
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

struct TempUpload {
    id: FileId,
    original_name: String,
    content_type: String,
    size: u64,
    content: TempUploadContent,
    preview: Option<String>,
    is_clipboard: bool,
}

enum TempUploadContent {
    Path(PathBuf),
    Bytes(Bytes),
}

struct TempPathGuard(Option<PathBuf>);

impl TempPathGuard {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn disarm(mut self) -> PathBuf {
        self.0.take().expect("temporary path guard should be armed")
    }
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl TempUpload {
    fn file_content(&self) -> FileContent {
        match &self.content {
            TempUploadContent::Path(path) => FileContent::Path(path.clone()),
            TempUploadContent::Bytes(bytes) => {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                tx.try_send(bytes.clone())
                    .expect("clipboard payload channel should have capacity");
                FileContent::Stream(rx)
            }
        }
    }
}

impl Drop for TempUpload {
    fn drop(&mut self) {
        if let TempUploadContent::Path(path) = &self.content {
            let _ = std::fs::remove_file(path);
        }
    }
}

async fn send_files(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<SendResponse>, ApiError> {
    let mut targets = Vec::new();
    let mut pin = None;
    let mut uploads = Vec::new();
    let mut total_bytes = 0_u64;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?
    {
        let field_name = field.name().unwrap_or_default().to_owned();
        match field_name.as_str() {
            "target" | "targets" => {
                if !uploads.is_empty() {
                    cleanup_uploads(&uploads).await;
                    return Err(ApiError::bad_request(
                        "Target devices must be provided before files",
                    ));
                }
                let value = match read_limited_text_field(&mut field, MAX_TARGET_FIELD_BYTES).await
                {
                    Ok(value) => value,
                    Err(error) => {
                        cleanup_uploads(&uploads).await;
                        return Err(error);
                    }
                };
                let parsed = if field_name == "targets" {
                    serde_json::from_str::<Vec<DeviceInfo>>(&value)
                } else {
                    serde_json::from_str::<DeviceInfo>(&value).map(|target| vec![target])
                };
                match parsed {
                    Ok(mut parsed) => {
                        if let Err(error) = validate_target_count(&parsed) {
                            cleanup_uploads(&uploads).await;
                            return Err(error);
                        }
                        deduplicate_targets(&mut parsed);
                        targets.extend(parsed);
                        deduplicate_targets(&mut targets);
                        if let Err(error) = validate_target_count(&targets) {
                            cleanup_uploads(&uploads).await;
                            return Err(error);
                        }
                    }
                    Err(error) => {
                        cleanup_uploads(&uploads).await;
                        return Err(ApiError::bad_request(format!(
                            "Invalid target devices: {error}"
                        )));
                    }
                }
            }
            "pin" => {
                let value = match read_limited_text_field(&mut field, MAX_PIN_FIELD_BYTES).await {
                    Ok(value) => value,
                    Err(error) => {
                        cleanup_uploads(&uploads).await;
                        return Err(error);
                    }
                };
                if !value.trim().is_empty() {
                    pin = Some(value);
                }
            }
            "files" => {
                if uploads.len() >= MAX_SEND_FILES {
                    cleanup_uploads(&uploads).await;
                    return Err(ApiError::bad_request(format!(
                        "Select no more than {MAX_SEND_FILES} files"
                    )));
                }
                if targets.is_empty() {
                    cleanup_uploads(&uploads).await;
                    return Err(ApiError::bad_request(
                        "The target device must be provided before files",
                    ));
                }
                if let Err(error) = validate_known_targets(&state, &targets) {
                    cleanup_uploads(&uploads).await;
                    return Err(error);
                }
                let original_name = safe_file_name(field.file_name().unwrap_or("file"));
                let content_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let id = FileId::new();
                let temp_path = state
                    .config
                    .temp_dir()
                    .join(format!("{TEMP_UPLOAD_PREFIX}{}", id.as_str()));
                let temp_guard = TempPathGuard::new(temp_path.clone());
                let mut output = match File::create(&temp_path).await {
                    Ok(output) => output,
                    Err(error) => {
                        cleanup_uploads(&uploads).await;
                        return Err(ApiError::internal(error));
                    }
                };
                let mut size = 0_u64;

                loop {
                    let chunk = match field.chunk().await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => break,
                        Err(error) => {
                            let _ = tokio::fs::remove_file(&temp_path).await;
                            cleanup_uploads(&uploads).await;
                            return Err(ApiError::bad_request(error.to_string()));
                        }
                    };
                    size = size
                        .checked_add(chunk.len() as u64)
                        .ok_or_else(|| ApiError::payload_too_large("Upload size overflow"))?;
                    total_bytes = total_bytes
                        .checked_add(chunk.len() as u64)
                        .ok_or_else(|| ApiError::payload_too_large("Upload size overflow"))?;
                    if total_bytes > state.config.max_upload_bytes {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        cleanup_uploads(&uploads).await;
                        return Err(ApiError::payload_too_large(format!(
                            "Upload exceeds the configured {} byte limit",
                            state.config.max_upload_bytes
                        )));
                    }
                    if let Err(error) = output.write_all(&chunk).await {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        cleanup_uploads(&uploads).await;
                        return Err(ApiError::internal(error));
                    }
                }
                if let Err(error) = output.flush().await {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    cleanup_uploads(&uploads).await;
                    return Err(ApiError::internal(error));
                }
                uploads.push(TempUpload {
                    id,
                    original_name,
                    content_type,
                    size,
                    content: TempUploadContent::Path(temp_guard.disarm()),
                    preview: None,
                    is_clipboard: false,
                });
            }
            _ => {}
        }
    }

    if uploads.is_empty() {
        return Err(ApiError::bad_request("Select at least one file"));
    }

    if targets.is_empty() {
        return Err(ApiError::bad_request("Select at least one target device"));
    }
    deduplicate_targets(&mut targets);
    validate_target_count(&targets)?;
    send_uploads(&state, targets, pin, uploads, total_bytes).await
}

async fn read_limited_text_field(
    field: &mut Field<'_>,
    max_bytes: usize,
) -> Result<String, ApiError> {
    let mut value = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?
    {
        if value.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ApiError::payload_too_large(
                "Multipart text field exceeds its configured limit",
            ));
        }
        value.extend_from_slice(&chunk);
    }
    String::from_utf8(value).map_err(ApiError::bad_request)
}

#[derive(Deserialize)]
struct SendTextRequest {
    #[serde(default)]
    targets: Vec<DeviceInfo>,
    target: Option<DeviceInfo>,
    text: String,
    pin: Option<String>,
}

async fn send_text(
    State(state): State<AppState>,
    Json(request): Json<SendTextRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    let mut targets = request.targets;
    if let Some(target) = request.target {
        targets.push(target);
    }
    validate_target_count(&targets)?;
    deduplicate_targets(&mut targets);
    if targets.is_empty() {
        return Err(ApiError::bad_request("Select at least one target device"));
    }
    validate_target_count(&targets)?;
    validate_known_targets(&state, &targets)?;
    let upload = build_text_upload(
        &request.text,
        state.config.max_upload_bytes.min(MAX_TEXT_BYTES),
    )?;
    let total_bytes = upload.size;
    send_uploads(
        &state,
        targets,
        request.pin.filter(|pin| !pin.trim().is_empty()),
        vec![upload],
        total_bytes,
    )
    .await
}

fn build_text_upload(text: &str, max_upload_bytes: u64) -> Result<TempUpload, ApiError> {
    if text.trim().is_empty() {
        return Err(ApiError::bad_request("Enter text to send"));
    }
    let size = u64::try_from(text.len()).map_err(ApiError::payload_too_large)?;
    if size > max_upload_bytes {
        return Err(ApiError::payload_too_large(format!(
            "Text exceeds the configured {max_upload_bytes} byte limit"
        )));
    }
    let id = FileId::new();
    Ok(TempUpload {
        original_name: format!("{}.txt", id.as_str()),
        content_type: "text/plain".to_owned(),
        size,
        content: TempUploadContent::Bytes(Bytes::copy_from_slice(text.as_bytes())),
        preview: Some(text.to_owned()),
        is_clipboard: true,
        id,
    })
}

async fn send_uploads(
    state: &AppState,
    targets: Vec<DeviceInfo>,
    pin: Option<String>,
    uploads: Vec<TempUpload>,
    total_bytes: u64,
) -> Result<Json<SendResponse>, ApiError> {
    let results = stream::iter(
        targets
            .into_iter()
            .map(|target| send_to_target(state, target, pin.as_deref(), &uploads, total_bytes)),
    )
    .buffer_unordered(MAX_CONCURRENT_SENDS)
    .collect::<Vec<_>>()
    .await;
    cleanup_uploads(&uploads).await;
    let transfer_id = results
        .first()
        .expect("target validation requires at least one device")
        .transfer_id;
    if results.iter().all(|result| result.files_sent == 0) {
        return Err(ApiError::bad_gateway(
            results
                .iter()
                .filter_map(|result| result.error.as_deref())
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let files_sent = results.iter().map(|result| result.files_sent).sum();
    let batch_total_bytes = results.iter().map(|result| result.total_bytes).sum();
    Ok(Json(SendResponse {
        transfer_id,
        transfers: results,
        files_sent,
        total_bytes: batch_total_bytes,
    }))
}

async fn send_to_target(
    state: &AppState,
    target: DeviceInfo,
    pin: Option<&str>,
    uploads: &[TempUpload],
    total_bytes: u64,
) -> SendTargetResponse {
    let _permit = state
        .send_semaphore
        .acquire()
        .await
        .expect("send semaphore should remain open");
    let transfer_id = Uuid::new_v4();
    let file_names = uploads
        .iter()
        .map(|upload| upload.original_name.clone())
        .collect::<Vec<_>>();
    let transferred_bytes = Arc::new(AtomicU64::new(0));
    let transfer = OutgoingTransfer {
        id: transfer_id,
        target_alias: target.alias.clone(),
        file_names,
        total_bytes,
        transferred_bytes: transferred_bytes.clone(),
        status: TransferStatus::Preparing,
        created_at: Utc::now(),
        error: None,
        content_type: uploads.first().map(|upload| upload.content_type.clone()),
        is_clipboard: uploads.first().is_some_and(|upload| upload.is_clipboard),
    };
    {
        let mut transfers = state.outgoing_transfers.write().await;
        if transfers.len() >= MAX_IN_MEMORY_TRANSFERS
            && let Some(index) = transfers.iter().position(|transfer| {
                matches!(
                    transfer.status,
                    TransferStatus::Completed | TransferStatus::Failed
                )
            })
        {
            transfers.remove(index);
        }
        transfers.push(transfer);
    }

    let result = perform_send(
        state,
        transfer_id,
        &target,
        pin,
        uploads,
        total_bytes,
        transferred_bytes.clone(),
    )
    .await;

    match result {
        Ok(outcome) => {
            let files_sent = outcome.accepted_ids.len();
            let all_accepted = files_sent == uploads.len();
            let partial_error = (!all_accepted)
                .then(|| format!("Receiver accepted {files_sent} of {} files", uploads.len()));
            if let Some(record) = state
                .outgoing_transfers
                .write()
                .await
                .iter_mut()
                .find(|record| record.id == transfer_id)
            {
                record.status = if all_accepted {
                    TransferStatus::Completed
                } else {
                    TransferStatus::Failed
                };
                record.total_bytes = outcome.total_bytes;
                record.error = partial_error.clone();
            }
            transferred_bytes.store(outcome.total_bytes, Ordering::Relaxed);
            let persistence_error = persist_outgoing_transfers(
                state,
                transfer_id,
                &target,
                uploads,
                Some(&outcome.accepted_ids),
                partial_error.as_deref(),
            )
            .err()
            .map(|error| error.message);
            let error = match (partial_error, persistence_error) {
                (Some(partial), Some(persistence)) => Some(format!("{partial}; {persistence}")),
                (Some(partial), None) => Some(partial),
                (None, Some(persistence)) => Some(persistence),
                (None, None) => None,
            };
            SendTargetResponse {
                transfer_id,
                target_alias: target.alias,
                files_sent,
                total_bytes: outcome.total_bytes,
                success: all_accepted,
                error,
            }
        }
        Err(failure) => {
            let error = failure.error;
            if let Some(record) = state
                .outgoing_transfers
                .write()
                .await
                .iter_mut()
                .find(|record| record.id == transfer_id)
            {
                record.status = TransferStatus::Failed;
                record.error = Some(error.message.clone());
            }
            transferred_bytes.store(failure.completed_bytes, Ordering::Relaxed);
            let persistence_result = persist_outgoing_transfers(
                state,
                transfer_id,
                &target,
                uploads,
                Some(&failure.completed_ids),
                Some(&error.message),
            );
            let message = match persistence_result {
                Ok(()) => error.message,
                Err(persistence_error) => {
                    format!("{}; {}", error.message, persistence_error.message)
                }
            };
            SendTargetResponse {
                transfer_id,
                target_alias: target.alias,
                files_sent: failure.completed_ids.len(),
                total_bytes: failure.completed_bytes,
                success: false,
                error: Some(message),
            }
        }
    }
}

fn persist_outgoing_transfers(
    state: &AppState,
    transfer_id: Uuid,
    target: &DeviceInfo,
    uploads: &[TempUpload],
    accepted_ids: Option<&HashSet<String>>,
    error: Option<&str>,
) -> Result<(), ApiError> {
    let records = outgoing_transfer_records(
        state.instance_key.instance_id(),
        transfer_id,
        target,
        uploads,
        accepted_ids,
        error,
    );
    state
        .database
        .record_transfers(&records)
        .map_err(ApiError::internal)
}

fn outgoing_transfer_records(
    instance_id: String,
    transfer_id: Uuid,
    target: &DeviceInfo,
    uploads: &[TempUpload],
    accepted_ids: Option<&HashSet<String>>,
    error: Option<&str>,
) -> Vec<TransferRecord> {
    let created_at_ms = Utc::now().timestamp_millis();
    let batch_id = transfer_id.to_string();
    uploads
        .iter()
        .map(|upload| {
            let completed = accepted_ids.is_some_and(|ids| ids.contains(upload.id.as_str()));
            TransferRecord {
                id: format!("{transfer_id}:{}", upload.id),
                batch_id: batch_id.clone(),
                instance_id: instance_id.clone(),
                direction: "outgoing".to_owned(),
                peer_alias: target.alias.clone(),
                file_name: upload.original_name.clone(),
                size: upload.size,
                status: if completed { "completed" } else { "failed" }.to_owned(),
                created_at_ms,
                error: (!completed).then(|| {
                    error
                        .unwrap_or("Receiver did not accept this file")
                        .to_owned()
                }),
                content_type: Some(upload.content_type.clone()),
                is_clipboard: upload.is_clipboard,
            }
        })
        .collect()
}

async fn perform_send(
    state: &AppState,
    transfer_id: Uuid,
    target: &DeviceInfo,
    pin: Option<&str>,
    uploads: &[TempUpload],
    total_bytes: u64,
    transferred_bytes: Arc<AtomicU64>,
) -> Result<SendOutcome, SendFailure> {
    let host = target
        .ip
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("The target device has no reachable address"))?;
    let protocol = target.protocol.into();
    let expected_fingerprint =
        (target.protocol == Protocol::Https).then(|| target.fingerprint.clone());
    let local_device = state
        .local_device
        .read()
        .expect("local device lock should not be poisoned")
        .clone();
    let client = LsHttpClientV2::try_new(
        &state.identity.material.private_key_pem,
        &state.identity.material.certificate_pem,
        expected_fingerprint.clone(),
        None,
    )
    .map_err(|error| ApiError::internal(error.to_string()))?;
    let registration = tokio::time::timeout(
        REGISTER_TIMEOUT,
        client.register(protocol, host, target.port, local_device.to_register()),
    )
    .await
    .map_err(|_| ApiError::bad_gateway("Target registration timed out"))?
    .map_err(|error| ApiError::bad_gateway(error.to_string()))?;

    let files = uploads
        .iter()
        .map(|upload| {
            let metadata = file_metadata(upload);
            (upload.id.0.clone(), metadata)
        })
        .collect::<HashMap<_, _>>();

    let prepared = tokio::time::timeout(
        PREPARE_TIMEOUT,
        client.prepare_upload(
            protocol,
            host,
            target.port,
            registration.public_key.clone(),
            PrepareUploadRequestDtoV2 {
                info: local_device.to_register(),
                files,
            },
            pin,
            CancellationToken::new(),
        ),
    )
    .await
    .map_err(|_| ApiError::bad_gateway("Target did not answer the transfer request"))?
    .map_err(|error| ApiError::bad_gateway(error.to_string()))?;
    let Some(prepared) = prepared.response else {
        if uploads.iter().all(|upload| upload.is_clipboard) {
            return Ok(SendOutcome {
                accepted_ids: uploads
                    .iter()
                    .map(|upload| upload.id.as_str().to_owned())
                    .collect(),
                total_bytes,
            });
        }
        return Err(ApiError::bad_gateway("Target accepted no files").into());
    };
    let session_id = prepared.session_id;
    let session_guard = RemoteSessionGuard::arm(
        state,
        protocol,
        host,
        target.port,
        &session_id,
        expected_fingerprint,
    );
    if let Some(transfer) = state
        .outgoing_transfers
        .write()
        .await
        .iter_mut()
        .find(|transfer| transfer.id == transfer_id)
    {
        transfer.status = TransferStatus::Sending;
    }
    let by_id = uploads
        .iter()
        .map(|upload| (upload.id.as_str(), upload))
        .collect::<HashMap<_, _>>();

    let accepted_ids = prepared.files.keys().cloned().collect::<HashSet<_>>();
    let accepted_total_bytes = accepted_ids.iter().try_fold(0_u64, |total, file_id| {
        let upload = by_id
            .get(file_id.as_str())
            .ok_or_else(|| ApiError::internal("Receiver accepted an unknown file"))?;
        Ok::<_, ApiError>(total.saturating_add(upload.size))
    });
    let accepted_total_bytes = match accepted_total_bytes {
        Ok(total) if !accepted_ids.is_empty() => total,
        Ok(_) => {
            session_guard.cancel().await;
            return Err(ApiError::bad_gateway("Target accepted no files").into());
        }
        Err(error) => {
            session_guard.cancel().await;
            return Err(error.into());
        }
    };
    if let Some(transfer) = state
        .outgoing_transfers
        .write()
        .await
        .iter_mut()
        .find(|transfer| transfer.id == transfer_id)
    {
        transfer.total_bytes = accepted_total_bytes;
    }

    let mut completed_ids = HashSet::new();
    let mut completed_bytes = 0_u64;
    for (file_id, token) in &prepared.files {
        let upload = by_id
            .get(file_id.as_str())
            .expect("accepted file IDs were validated");
        let progress = transferred_bytes.clone();
        let base = completed_bytes;
        let upload_timeout =
            Duration::from_secs(120_u64.saturating_add(upload.size.saturating_div(1024 * 1024)));
        let result = tokio::time::timeout(
            upload_timeout,
            client.upload(
                protocol,
                host,
                target.port,
                registration.public_key.clone(),
                &session_id,
                file_id,
                token,
                upload_body(upload.file_content(), move |sent| {
                    progress.store(
                        base.saturating_add(sent).min(accepted_total_bytes),
                        Ordering::Relaxed,
                    );
                }),
                CancellationToken::new(),
            ),
        )
        .await
        .map_err(|_| ApiError::bad_gateway("File upload timed out"))
        .and_then(|result| result.map_err(|error| ApiError::bad_gateway(error.to_string())));
        if let Err(error) = result {
            session_guard.cancel().await;
            return Err(SendFailure {
                error,
                completed_ids,
                completed_bytes,
            });
        }
        completed_ids.insert(file_id.clone());
        completed_bytes = completed_bytes.saturating_add(upload.size);
        transferred_bytes.store(completed_bytes.min(accepted_total_bytes), Ordering::Relaxed);
    }
    session_guard.complete().await;

    Ok(SendOutcome {
        accepted_ids: completed_ids,
        total_bytes: accepted_total_bytes,
    })
}

fn file_metadata(upload: &TempUpload) -> FileDto {
    FileDto {
        id: upload.id.0.clone(),
        file_name: upload.original_name.clone(),
        size: upload.size,
        file_type: upload.content_type.clone(),
        sha256: None,
        preview: upload.preview.clone(),
        metadata: None,
    }
}

fn upload_body(content: FileContent, progress: impl Fn(u64) + Send + 'static) -> reqwest::Body {
    let mut sent = 0_u64;
    let stream = ReceiverStream::new(content.into_receiver()).map(move |chunk| {
        sent = sent.saturating_add(chunk.len() as u64);
        progress(sent);
        Ok::<Bytes, anyhow::Error>(chunk)
    });
    reqwest::Body::wrap_stream(stream)
}

fn deduplicate_targets(targets: &mut Vec<DeviceInfo>) {
    let mut seen = HashSet::new();
    targets
        .retain(|target| seen.insert((target.fingerprint.clone(), target.ip.clone(), target.port)));
}

fn validate_target_count(targets: &[DeviceInfo]) -> Result<(), ApiError> {
    if targets.len() > MAX_SEND_TARGETS {
        return Err(ApiError::bad_request(format!(
            "Select no more than {MAX_SEND_TARGETS} target devices"
        )));
    }
    Ok(())
}

fn validate_known_targets(state: &AppState, targets: &[DeviceInfo]) -> Result<(), ApiError> {
    let known = state.active_devices();
    if targets.iter().all(|target| {
        known.iter().any(|candidate| {
            candidate.device.fingerprint == target.fingerprint
                && candidate.device.ip == target.ip
                && candidate.device.port == target.port
                && candidate.device.protocol == target.protocol
        })
    }) {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "Refresh discovery or connect to each target before sending",
    ))
}

fn safe_file_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("file")
        .to_owned()
}

async fn cleanup_uploads(uploads: &[TempUpload]) {
    for upload in uploads {
        if let TempUploadContent::Path(path) = &upload.content {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

pub(crate) async fn cleanup_stale_temp_uploads(temp_dir: &Path) -> anyhow::Result<()> {
    let mut entries = match tokio::fs::read_dir(temp_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(id) = name.strip_prefix(TEMP_UPLOAD_PREFIX)
            && Uuid::parse_str(id).is_ok()
        {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl ToString) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn not_found(message: impl ToString) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn conflict(message: impl ToString) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    fn payload_too_large(message: impl ToString) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, message)
    }

    fn bad_gateway(message: impl ToString) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message)
    }

    fn internal(message: impl ToString) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    fn new(status: StatusCode, message: impl ToString) -> Self {
        Self {
            status,
            message: message.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorBody {
            error: String,
        }

        (
            StatusCode::from_u16(self.status.as_u16()).unwrap(),
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        net::{IpAddr, Ipv4Addr},
    };

    use super::{
        MAX_SEND_TARGETS, TEMP_UPLOAD_PREFIX, TempUpload, TempUploadContent, build_text_upload,
        cleanup_stale_temp_uploads, deduplicate_targets, file_metadata, outgoing_transfer_records,
        parse_probe_address, queue_discovery_scan, safe_file_name, validate_target_count,
    };
    use crate::network::DiscoveryCommand;
    use localsendy_core::{DeviceInfo, Protocol};
    use tokio::sync::mpsc;

    #[test]
    fn coalesces_duplicate_discovery_scans() {
        let (scan_tx, mut scan_rx) = mpsc::channel(1);

        queue_discovery_scan(&scan_tx).unwrap();
        queue_discovery_scan(&scan_tx).unwrap();

        assert!(matches!(scan_rx.try_recv(), Ok(DiscoveryCommand::Announce)));
        assert!(scan_rx.try_recv().is_err());
    }

    #[test]
    fn clipboard_text_uses_localsend_message_metadata() {
        let upload = build_text_upload("Hello 局域网", 1024).unwrap();
        let metadata = file_metadata(&upload);

        assert_eq!(metadata.file_type, "text/plain");
        assert_eq!(metadata.preview.as_deref(), Some("Hello 局域网"));
        assert!(metadata.file_name.ends_with(".txt"));
        let TempUploadContent::Bytes(content) = &upload.content else {
            panic!("clipboard text should remain in memory");
        };
        assert_eq!(content.as_ref(), "Hello 局域网".as_bytes());
    }

    #[test]
    fn staged_upload_is_removed_when_its_guard_drops() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("staged-file");
        std::fs::write(&path, b"payload").unwrap();
        let upload = TempUpload {
            id: localsendy_core::FileId::new(),
            original_name: "payload.bin".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            size: 7,
            content: TempUploadContent::Path(path.clone()),
            preview: None,
            is_clipboard: false,
        };

        drop(upload);
        assert!(!path.exists());
    }

    #[test]
    fn outgoing_batch_preserves_files_completed_before_a_later_failure() {
        let first = build_text_upload("first", 1024).unwrap();
        let second = build_text_upload("second", 1024).unwrap();
        let completed = HashSet::from([first.id.as_str().to_owned()]);
        let target = DeviceInfo {
            alias: "Phone".to_owned(),
            version: "2.1".to_owned(),
            device_model: None,
            device_type: None,
            fingerprint: "fingerprint".to_owned(),
            port: 53317,
            protocol: Protocol::Https,
            download: false,
            ip: Some("192.168.1.10".to_owned()),
        };

        let records = outgoing_transfer_records(
            "single".to_owned(),
            uuid::Uuid::new_v4(),
            &target,
            &[first, second],
            Some(&completed),
            Some("second upload failed"),
        );

        assert_eq!(records[0].status, "completed");
        assert!(records[0].error.is_none());
        assert_eq!(records[1].status, "failed");
        assert_eq!(records[1].error.as_deref(), Some("second upload failed"));
    }

    #[tokio::test]
    async fn startup_cleanup_only_removes_owned_temp_uploads() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory
            .path()
            .join(format!("{TEMP_UPLOAD_PREFIX}{}", uuid::Uuid::new_v4()));
        let legacy = directory.path().join(uuid::Uuid::new_v4().to_string());
        let unrelated = directory.path().join("keep-me.txt");
        tokio::fs::write(&current, b"current").await.unwrap();
        tokio::fs::write(&legacy, b"legacy").await.unwrap();
        tokio::fs::write(&unrelated, b"unrelated").await.unwrap();

        cleanup_stale_temp_uploads(directory.path()).await.unwrap();

        assert!(!current.exists());
        assert!(legacy.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn strips_untrusted_path_components() {
        assert_eq!(safe_file_name("../../secret.txt"), "secret.txt");
        assert_eq!(safe_file_name(""), "file");
    }

    #[test]
    fn parses_manual_device_addresses() {
        assert_eq!(
            parse_probe_address("192.168.1.50").unwrap(),
            (IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)), 53317)
        );
        assert_eq!(parse_probe_address("192.168.1.50:54000").unwrap().1, 54000);
        assert!(parse_probe_address("example.com").is_err());
    }

    #[test]
    fn deduplicates_only_identical_device_endpoints() {
        let device = DeviceInfo {
            alias: "Phone".to_owned(),
            version: "2.1".to_owned(),
            device_model: None,
            device_type: None,
            fingerprint: "fingerprint".to_owned(),
            port: 53317,
            protocol: Protocol::Https,
            download: false,
            ip: Some("192.168.1.10".to_owned()),
        };
        let mut devices = vec![
            device.clone(),
            device.clone(),
            DeviceInfo {
                port: 53318,
                ..device
            },
        ];

        deduplicate_targets(&mut devices);

        assert_eq!(devices.len(), 2);
    }

    #[test]
    fn rejects_unbounded_multi_device_batches() {
        let device = DeviceInfo {
            alias: "Phone".to_owned(),
            version: "2.1".to_owned(),
            device_model: None,
            device_type: None,
            fingerprint: "fingerprint".to_owned(),
            port: 53317,
            protocol: Protocol::Https,
            download: false,
            ip: Some("192.168.1.10".to_owned()),
        };
        let targets = vec![device; MAX_SEND_TARGETS + 1];

        assert!(validate_target_count(&targets).is_err());
    }
}
