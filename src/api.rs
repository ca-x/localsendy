use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::Path,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use localsend_rs::{DeviceInfo, FileId, FileMetadata, LocalSendClient, Protocol};
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncWriteExt};
use uuid::Uuid;

use crate::network::{
    DiscoveryCommand, NetworkMode, NetworkPreferences, NetworkSelection, NetworkSettings,
    network_settings, route_interface_for_ip, save_preferences,
};
use crate::state::{AppState, DiscoveredDevice, OutgoingTransfer, SeenDevice, TransferStatus};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/devices", get(devices))
        .route("/devices/scan", post(scan))
        .route("/devices/probe", post(probe_device))
        .route("/networks", get(networks).put(update_networks))
        .route("/pending", get(pending))
        .route("/pending/{decision}", post(decide_pending))
        .route("/history", get(history))
        .route("/transfers", get(transfers))
        .route("/send", post(send_files))
        .layer(DefaultBodyLimit::disable())
        .with_state(state)
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    alias: String,
    web_address: String,
    localsend_port: u16,
    protocol: String,
    data_directory: String,
    auto_accept: bool,
    uptime_seconds: u64,
    nearby_devices: usize,
}

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let devices = state.active_devices();
    Json(StatusResponse {
        alias: state.config.alias.clone(),
        web_address: state.config.web_bind.to_string(),
        localsend_port: state.config.localsend_port,
        protocol: state.local_device.protocol.to_string(),
        data_directory: state.config.downloads_dir().display().to_string(),
        auto_accept: state.config.auto_accept,
        uptime_seconds: state.started_at.elapsed().as_secs(),
        nearby_devices: devices.len(),
    })
}

async fn devices(State(state): State<AppState>) -> Json<Vec<DiscoveredDevice>> {
    Json(state.active_devices())
}

async fn scan(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state
        .scan_tx
        .try_send(DiscoveryCommand::Announce)
        .map_err(|_| ApiError::conflict("A discovery scan is already queued"))?;
    Ok(StatusCode::ACCEPTED)
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
    let host = match ip {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => format!("[{address}]"),
    };
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(ApiError::internal)?;
    let mut last_error = None;

    for protocol in [Protocol::Https, Protocol::Http] {
        let url = format!("{}://{host}:{port}/api/localsend/v2/info", protocol);
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                let mut device = response
                    .json::<DeviceInfo>()
                    .await
                    .map_err(|error| ApiError::bad_gateway(error.to_string()))?;
                device.ip = Some(ip.to_string());
                device.protocol = protocol;
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
            Ok(response) => {
                last_error = Some(format!("{} returned {}", url, response.status()));
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
    Ok((ip, localsend_rs::DEFAULT_HTTP_PORT))
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

async fn history(State(state): State<AppState>) -> Json<Vec<localsend_rs::ReceivedFile>> {
    let mut files = state.received_files.read().await.clone();
    files.reverse();
    Json(files)
}

async fn transfers(State(state): State<AppState>) -> Json<Vec<OutgoingTransfer>> {
    let mut transfers = state.outgoing_transfers.read().await.clone();
    transfers.reverse();
    Json(transfers)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendResponse {
    transfer_id: Uuid,
    files_sent: usize,
    total_bytes: u64,
}

struct TempUpload {
    id: FileId,
    original_name: String,
    content_type: String,
    size: u64,
    path: std::path::PathBuf,
}

async fn send_files(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<SendResponse>, ApiError> {
    let mut target = None;
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
            "target" => {
                let value = match field.text().await {
                    Ok(value) => value,
                    Err(error) => {
                        cleanup_uploads(&uploads).await;
                        return Err(ApiError::bad_request(error.to_string()));
                    }
                };
                target = match serde_json::from_str::<DeviceInfo>(&value) {
                    Ok(device) => Some(device),
                    Err(error) => {
                        cleanup_uploads(&uploads).await;
                        return Err(ApiError::bad_request(format!(
                            "Invalid target device: {error}"
                        )));
                    }
                };
            }
            "pin" => {
                let value = field
                    .text()
                    .await
                    .map_err(|error| ApiError::bad_request(error.to_string()))?;
                if !value.trim().is_empty() {
                    pin = Some(value);
                }
            }
            "files" => {
                if target.is_none() {
                    cleanup_uploads(&uploads).await;
                    return Err(ApiError::bad_request(
                        "The target device must be provided before files",
                    ));
                }
                let original_name = safe_file_name(field.file_name().unwrap_or("file"));
                let content_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let id = FileId::new();
                let temp_path = state.config.temp_dir().join(id.as_str());
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
                    path: temp_path,
                });
            }
            _ => {}
        }
    }

    if uploads.is_empty() {
        return Err(ApiError::bad_request("Select at least one file"));
    }

    let target = target.ok_or_else(|| ApiError::bad_request("Missing target device"))?;
    let transfer_id = Uuid::new_v4();
    let file_names = uploads
        .iter()
        .map(|upload| upload.original_name.clone())
        .collect::<Vec<_>>();
    let transfer = OutgoingTransfer {
        id: transfer_id,
        target_alias: target.alias.clone(),
        file_names,
        total_bytes,
        status: TransferStatus::Preparing,
        created_at: Utc::now(),
        error: None,
    };
    state.outgoing_transfers.write().await.push(transfer);

    let result = perform_send(&state, &target, pin.as_deref(), &uploads).await;
    cleanup_uploads(&uploads).await;

    let mut records = state.outgoing_transfers.write().await;
    let record = records
        .iter_mut()
        .find(|record| record.id == transfer_id)
        .expect("newly inserted transfer record should exist");

    match result {
        Ok(files_sent) => {
            record.status = TransferStatus::Completed;
            Ok(Json(SendResponse {
                transfer_id,
                files_sent,
                total_bytes,
            }))
        }
        Err(error) => {
            record.status = TransferStatus::Failed;
            record.error = Some(error.message.clone());
            Err(error)
        }
    }
}

async fn perform_send(
    state: &AppState,
    target: &DeviceInfo,
    pin: Option<&str>,
    uploads: &[TempUpload],
) -> Result<usize, ApiError> {
    let target = normalize_target_for_client(target);
    let client = LocalSendClient::new(state.local_device.clone());
    let _ = client.register(&target).await;

    let files = uploads
        .iter()
        .map(|upload| {
            let metadata = FileMetadata {
                id: upload.id.clone(),
                file_name: upload.original_name.clone(),
                size: upload.size,
                file_type: upload.content_type.clone(),
                sha256: None,
                preview: None,
                metadata: None,
            };
            (upload.id.clone(), metadata)
        })
        .collect::<HashMap<_, _>>();

    let prepared = client
        .prepare_upload(&target, files, pin)
        .await
        .map_err(|error| ApiError::bad_gateway(error.to_string()))?;
    let by_id = uploads
        .iter()
        .map(|upload| (upload.id.as_str(), upload))
        .collect::<HashMap<_, _>>();

    for (file_id, token) in &prepared.files {
        let upload = by_id
            .get(file_id.as_str())
            .ok_or_else(|| ApiError::internal("Receiver accepted an unknown file"))?;
        client
            .upload_file(
                &target,
                &prepared.session_id,
                file_id,
                token,
                &upload.path,
                None,
            )
            .await
            .map_err(|error| ApiError::bad_gateway(error.to_string()))?;
    }

    Ok(prepared.files.len())
}

fn normalize_target_for_client(target: &DeviceInfo) -> DeviceInfo {
    let mut target = target.clone();
    if let Some(ip) = target.ip.as_deref()
        && ip.contains(':')
        && !ip.starts_with('[')
    {
        target.ip = Some(format!("[{}]", ip.replace('%', "%25")));
    }
    target
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
        let _ = tokio::fs::remove_file(&upload.path).await;
    }
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
    use std::net::{IpAddr, Ipv4Addr};

    use super::{parse_probe_address, safe_file_name};

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
}
