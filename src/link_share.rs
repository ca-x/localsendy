use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{
        ConnectInfo, DefaultBodyLimit, Multipart, Path as AxumPath, Query, State, multipart::Field,
    },
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use localsend::http::dto_v2::{InfoResponseDtoV2, PrepareDownloadResponseDtoV2};
use localsend::model::{discovery::DeviceType as LocalSendDeviceType, transfer::FileDto};
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncWriteExt, sync::oneshot};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::state::AppState;

const MAX_SHARE_FILES: usize = 100;
const MAX_SHARE_FIELDS: usize = MAX_SHARE_FILES + 3;
const MAX_PIN_BYTES: usize = 128;
const MAX_BOOLEAN_BYTES: usize = 5;
const MAX_SHARE_URL_BYTES: usize = 2048;
const MAX_PIN_ATTEMPTS: u32 = 10;
const SHARE_TEMP_PREFIX: &str = "share-";
const DOWNLOAD_HTML: &str = include_str!("../third_party/localsend-core/assets/web/download.html");

#[derive(Clone, Default)]
pub struct LinkShareStore(Arc<tokio::sync::RwLock<Option<ActiveLinkShare>>>);

struct ActiveLinkShare {
    share_id: Uuid,
    share_url: String,
    files: HashMap<String, SharedFile>,
    total_bytes: u64,
    auto_accept: bool,
    pin: Option<String>,
    requests: HashMap<String, ShareRequest>,
    pin_attempts: HashMap<IpAddr, u32>,
    created_at: String,
}

struct SharedFile {
    dto: FileDto,
    path: PathBuf,
}

struct ShareRequest {
    request_id: Uuid,
    ip: IpAddr,
    user_agent: Option<String>,
    status: ShareRequestStatus,
    decision_tx: Option<oneshot::Sender<bool>>,
    created_at: String,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum ShareRequestStatus {
    Pending,
    Accepted,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkShareResponse {
    active: bool,
    share_id: Option<Uuid>,
    urls: Vec<String>,
    files: Vec<SharedFileResponse>,
    total_bytes: u64,
    auto_accept: bool,
    pin: Option<String>,
    requests: Vec<ShareRequestResponse>,
    created_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedFileResponse {
    id: String,
    name: String,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareRequestResponse {
    session_id: String,
    ip: String,
    user_agent: Option<String>,
    status: ShareRequestStatus,
    created_at: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareSettingsRequest {
    auto_accept: bool,
    pin: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadQuery {
    session_id: Option<String>,
    file_id: Option<String>,
    pin: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StopShareQuery {
    share_id: Option<Uuid>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadI18n {
    waiting: &'static str,
    enter_pin: &'static str,
    invalid_pin: &'static str,
    too_many_attempts: &'static str,
    rejected: &'static str,
    files: &'static str,
}

pub fn control_router() -> Router<AppState> {
    Router::new()
        .route(
            "/share",
            get(get_share)
                .put(update_share)
                .delete(stop_share)
                .merge(post(start_share).layer(DefaultBodyLimit::disable())),
        )
        .route(
            "/share/requests/{session_id}/{decision}",
            post(decide_request),
        )
}

pub fn public_router(state: AppState) -> Router {
    Router::new()
        .route("/share", get(download_page))
        .route("/share/", get(download_page))
        .route("/share/i18n.json", get(download_i18n))
        .route("/share/api/prepare-download", post(prepare_download))
        .route("/share/api/download", get(download_file))
        .with_state(state)
}

async fn get_share(State(state): State<AppState>) -> Json<LinkShareResponse> {
    Json(share_response(&state).await)
}

async fn start_share(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<LinkShareResponse>, ShareError> {
    let mut auto_accept = false;
    let mut pin = None;
    let mut share_url = None;
    let mut files = HashMap::new();
    let mut total_bytes = 0_u64;
    let mut staged_files = StagedShareFiles::default();
    let mut field_count = 0_usize;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|error| ShareError::bad_request(error.to_string()))?
    {
        field_count += 1;
        if field_count > MAX_SHARE_FIELDS {
            return Err(ShareError::bad_request("Too many multipart fields"));
        }
        match field.name().unwrap_or_default() {
            "autoAccept" => {
                let value = read_text_field(&mut field, MAX_BOOLEAN_BYTES).await?;
                auto_accept = value == "true";
            }
            "pin" => {
                let value = read_text_field(&mut field, MAX_PIN_BYTES).await?;
                pin = normalize_pin(value);
            }
            "shareUrl" => {
                let value = read_text_field(&mut field, MAX_SHARE_URL_BYTES).await?;
                share_url = Some(normalize_share_url(&value)?);
            }
            "files" => {
                if files.len() >= MAX_SHARE_FILES {
                    cleanup_files(files.values()).await;
                    return Err(ShareError::bad_request(format!(
                        "Select no more than {MAX_SHARE_FILES} files"
                    )));
                }
                let id = Uuid::new_v4().to_string();
                let file_name = safe_file_name(field.file_name().unwrap_or("file"));
                let file_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let path = state
                    .config
                    .temp_dir()
                    .join(format!("{SHARE_TEMP_PREFIX}{id}"));
                let mut output = match File::create(&path).await {
                    Ok(output) => output,
                    Err(error) => {
                        cleanup_files(files.values()).await;
                        return Err(ShareError::internal(error));
                    }
                };
                staged_files.paths.push(path.clone());
                let mut size = 0_u64;
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|error| ShareError::bad_request(error.to_string()))?
                {
                    size = size
                        .checked_add(chunk.len() as u64)
                        .ok_or_else(|| ShareError::too_large("Share size overflow"))?;
                    total_bytes = total_bytes
                        .checked_add(chunk.len() as u64)
                        .ok_or_else(|| ShareError::too_large("Share size overflow"))?;
                    if total_bytes > state.config.max_upload_bytes {
                        let _ = tokio::fs::remove_file(&path).await;
                        cleanup_files(files.values()).await;
                        return Err(ShareError::too_large(format!(
                            "Shared files exceed the configured limit of {} bytes",
                            state.config.max_upload_bytes
                        )));
                    }
                    if let Err(error) = output.write_all(&chunk).await {
                        let _ = tokio::fs::remove_file(&path).await;
                        cleanup_files(files.values()).await;
                        return Err(ShareError::internal(error));
                    }
                }
                output.flush().await.map_err(ShareError::internal)?;
                files.insert(
                    id.clone(),
                    SharedFile {
                        dto: FileDto {
                            id,
                            file_name,
                            size,
                            file_type,
                            sha256: None,
                            preview: None,
                            metadata: None,
                        },
                        path,
                    },
                );
            }
            _ => return Err(ShareError::bad_request("Unknown multipart field")),
        }
    }

    if files.is_empty() {
        return Err(ShareError::bad_request("Select at least one file"));
    }
    let share_url = share_url.ok_or_else(|| ShareError::bad_request("Missing share URL"))?;

    let active = ActiveLinkShare {
        share_id: Uuid::new_v4(),
        share_url,
        files,
        total_bytes,
        auto_accept,
        pin,
        requests: HashMap::new(),
        pin_attempts: HashMap::new(),
        created_at: Utc::now().to_rfc3339(),
    };
    replace_share(&state.link_share, Some(active)).await;
    staged_files.disarm();
    Ok(Json(share_response(&state).await))
}

async fn update_share(
    State(state): State<AppState>,
    Json(settings): Json<ShareSettingsRequest>,
) -> Result<Json<LinkShareResponse>, ShareError> {
    if settings.pin.len() > MAX_PIN_BYTES {
        return Err(ShareError::bad_request("PIN is too long"));
    }
    let pin = normalize_pin(settings.pin);
    let mut store = state.link_share.0.write().await;
    let active = store
        .as_mut()
        .ok_or_else(|| ShareError::not_found("No active link share"))?;
    active.auto_accept = settings.auto_accept;
    if active.pin != pin {
        active.pin = pin;
        active.pin_attempts.clear();
        reject_all_requests(active);
    }
    drop(store);
    Ok(Json(share_response(&state).await))
}

async fn stop_share(
    State(state): State<AppState>,
    Query(query): Query<StopShareQuery>,
) -> Result<StatusCode, ShareError> {
    let share_id = query
        .share_id
        .ok_or_else(|| ShareError::bad_request("Missing shareId"))?;
    let previous = take_matching_share(&state.link_share, share_id)
        .await
        .ok_or_else(|| ShareError::not_found("Link share not found"))?;
    cleanup_share(Some(previous)).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn decide_request(
    State(state): State<AppState>,
    AxumPath((session_id, decision)): AxumPath<(String, String)>,
) -> Result<StatusCode, ShareError> {
    let accepted = match decision.as_str() {
        "accept" => true,
        "reject" => false,
        _ => return Err(ShareError::bad_request("Decision must be accept or reject")),
    };
    let mut store = state.link_share.0.write().await;
    let active = store
        .as_mut()
        .ok_or_else(|| ShareError::not_found("No active link share"))?;
    let request = active
        .requests
        .get_mut(&session_id)
        .ok_or_else(|| ShareError::not_found("Download request not found"))?;
    if accepted {
        request.status = ShareRequestStatus::Accepted;
    }
    if let Some(decision_tx) = request.decision_tx.take() {
        let _ = decision_tx.send(accepted);
    }
    if !accepted {
        active.requests.remove(&session_id);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn download_page(State(state): State<AppState>) -> Response {
    if state.link_share.0.read().await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            "This link share is no longer active.",
        )
            .into_response();
    }
    let page = DOWNLOAD_HTML
        .replace(
            "var BASE_URL = '/api/localsend/v2';",
            "var BASE_URL = '/share/api';",
        )
        .replace("makeRequest('/i18n.json'", "makeRequest('/share/i18n.json'");
    let mut response = Html(page).into_response();
    no_store(&mut response);
    response
}

async fn download_i18n(State(state): State<AppState>) -> Result<Json<DownloadI18n>, ShareError> {
    if state.link_share.0.read().await.is_none() {
        return Err(ShareError::not_found("This link share is no longer active"));
    }
    Ok(Json(DownloadI18n {
        waiting: "Waiting for response…",
        enter_pin: "Enter PIN",
        invalid_pin: "Invalid PIN",
        too_many_attempts: "Too many attempts",
        rejected: "Rejected",
        files: "Files",
    }))
}

async fn prepare_download(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(query): Query<DownloadQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<PrepareDownloadResponseDtoV2>, ShareError> {
    let ip = peer.ip();
    {
        let store = state.link_share.0.read().await;
        let active = store
            .as_ref()
            .ok_or_else(|| ShareError::not_found("This link share is no longer active"))?;
        if let Some(session_id) = &query.session_id
            && active.requests.get(session_id).is_some_and(|request| {
                request.ip == ip && matches!(request.status, ShareRequestStatus::Accepted)
            })
        {
            return Ok(Json(download_response(&state, active, session_id.clone())));
        }
    }

    let session_id = ip.to_string();
    let request_id = Uuid::new_v4();
    let (decision_tx, decision_rx) = oneshot::channel();
    let auto_accept = {
        let mut store = state.link_share.0.write().await;
        let active = store
            .as_mut()
            .ok_or_else(|| ShareError::not_found("This link share is no longer active"))?;
        check_pin(active, ip, query.pin.as_deref())?;
        if let Some(previous) = active.requests.remove(&session_id)
            && let Some(previous_tx) = previous.decision_tx
        {
            let _ = previous_tx.send(false);
        }
        let auto_accept = active.auto_accept;
        active.requests.insert(
            session_id.clone(),
            ShareRequest {
                request_id,
                ip,
                user_agent: headers
                    .get(header::USER_AGENT)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
                status: if auto_accept {
                    ShareRequestStatus::Accepted
                } else {
                    ShareRequestStatus::Pending
                },
                decision_tx: (!auto_accept).then_some(decision_tx),
                created_at: Utc::now().to_rfc3339(),
            },
        );
        auto_accept
    };

    let mut pending_guard = (!auto_accept).then(|| {
        PendingShareRequestGuard::new(state.link_share.clone(), session_id.clone(), request_id)
    });
    if !auto_accept {
        if !decision_rx.await.unwrap_or(false) {
            if let Some(guard) = pending_guard.as_mut() {
                guard.clear().await;
            }
            return Err(ShareError::forbidden("File transfer rejected"));
        }
        if let Some(guard) = pending_guard.as_mut() {
            guard.disarm();
        }
    }

    let store = state.link_share.0.read().await;
    let active = store
        .as_ref()
        .ok_or_else(|| ShareError::not_found("This link share is no longer active"))?;
    let valid = active.requests.get(&session_id).is_some_and(|request| {
        request.ip == ip && matches!(request.status, ShareRequestStatus::Accepted)
    });
    if !valid {
        return Err(ShareError::forbidden("File transfer rejected"));
    }
    Ok(Json(download_response(&state, active, session_id)))
}

async fn download_file(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(query): Query<DownloadQuery>,
) -> Result<Response, ShareError> {
    let session_id = query
        .session_id
        .ok_or_else(|| ShareError::bad_request("Missing sessionId"))?;
    let file_id = query
        .file_id
        .ok_or_else(|| ShareError::bad_request("Missing fileId"))?;
    let (path, name, size) = {
        let store = state.link_share.0.read().await;
        let active = store
            .as_ref()
            .ok_or_else(|| ShareError::not_found("This link share is no longer active"))?;
        let valid = active.requests.get(&session_id).is_some_and(|request| {
            request.ip == peer.ip() && matches!(request.status, ShareRequestStatus::Accepted)
        });
        if !valid {
            return Err(ShareError::forbidden("Invalid sessionId"));
        }
        let shared = active
            .files
            .get(&file_id)
            .ok_or_else(|| ShareError::forbidden("Invalid fileId"))?;
        (
            shared.path.clone(),
            shared.dto.file_name.clone(),
            shared.dto.size,
        )
    };

    let file = File::open(path).await.map_err(ShareError::internal)?;
    let mut response = Response::new(Body::from_stream(ReaderStream::new(file)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&size.to_string()).map_err(ShareError::internal)?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(&name)).map_err(ShareError::internal)?,
    );
    no_store(&mut response);
    Ok(response)
}

fn download_response(
    state: &AppState,
    active: &ActiveLinkShare,
    session_id: String,
) -> PrepareDownloadResponseDtoV2 {
    let local = state
        .local_device
        .read()
        .expect("local device lock should not be poisoned");
    PrepareDownloadResponseDtoV2 {
        info: InfoResponseDtoV2 {
            alias: local.alias.clone(),
            version: local.version.clone(),
            device_model: local.device_model.clone(),
            device_type: local.device_type.map(|device_type| match device_type {
                localsendy_core::DeviceType::Mobile => LocalSendDeviceType::Mobile,
                localsendy_core::DeviceType::Desktop => LocalSendDeviceType::Desktop,
                localsendy_core::DeviceType::Web => LocalSendDeviceType::Web,
                localsendy_core::DeviceType::Headless => LocalSendDeviceType::Headless,
                localsendy_core::DeviceType::Server => LocalSendDeviceType::Server,
            }),
            fingerprint: local.fingerprint.clone(),
            download: true,
        },
        session_id,
        files: active
            .files
            .iter()
            .map(|(id, file)| (id.clone(), file.dto.clone()))
            .collect(),
    }
}

async fn share_response(state: &AppState) -> LinkShareResponse {
    let store = state.link_share.0.read().await;
    let Some(active) = store.as_ref() else {
        return LinkShareResponse {
            active: false,
            share_id: None,
            urls: Vec::new(),
            files: Vec::new(),
            total_bytes: 0,
            auto_accept: false,
            pin: None,
            requests: Vec::new(),
            created_at: None,
        };
    };
    let mut files = active
        .files
        .iter()
        .map(|(id, file)| SharedFileResponse {
            id: id.clone(),
            name: file.dto.file_name.clone(),
            size: file.dto.size,
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.name.cmp(&right.name));
    let mut requests = active
        .requests
        .iter()
        .map(|(session_id, request)| ShareRequestResponse {
            session_id: session_id.clone(),
            ip: request.ip.to_string(),
            user_agent: request.user_agent.clone(),
            status: request.status,
            created_at: request.created_at.clone(),
        })
        .collect::<Vec<_>>();
    requests.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    LinkShareResponse {
        active: true,
        share_id: Some(active.share_id),
        urls: vec![active.share_url.clone()],
        files,
        total_bytes: active.total_bytes,
        auto_accept: active.auto_accept,
        pin: active.pin.clone(),
        requests,
        created_at: Some(active.created_at.clone()),
    }
}

fn normalize_share_url(value: &str) -> Result<String, ShareError> {
    let mut url = reqwest::Url::parse(value.trim())
        .map_err(|_| ShareError::bad_request("Share URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ShareError::bad_request(
            "Share URL must be an HTTP(S) origin",
        ));
    }
    url.set_path("/share");
    Ok(url.to_string())
}

fn check_pin(
    active: &mut ActiveLinkShare,
    ip: IpAddr,
    supplied: Option<&str>,
) -> Result<(), ShareError> {
    let Some(expected) = active.pin.as_deref() else {
        return Ok(());
    };
    let attempts = active.pin_attempts.entry(ip).or_default();
    if *attempts >= MAX_PIN_ATTEMPTS {
        return Err(ShareError::too_many("Too many PIN attempts"));
    }
    if supplied != Some(expected) {
        *attempts += 1;
        return Err(ShareError::unauthorized("Invalid PIN"));
    }
    active.pin_attempts.remove(&ip);
    Ok(())
}

async fn replace_share(store: &LinkShareStore, next: Option<ActiveLinkShare>) {
    let previous = std::mem::replace(&mut *store.0.write().await, next);
    cleanup_share(previous).await;
}

async fn cleanup_share(previous: Option<ActiveLinkShare>) {
    if let Some(mut previous) = previous {
        reject_all_requests(&mut previous);
        cleanup_files(previous.files.values()).await;
    }
}

fn reject_all_requests(active: &mut ActiveLinkShare) {
    for (_, mut request) in active.requests.drain() {
        if let Some(decision_tx) = request.decision_tx.take() {
            let _ = decision_tx.send(false);
        }
    }
}

async fn cleanup_files<'a>(files: impl Iterator<Item = &'a SharedFile>) {
    for file in files {
        let _ = tokio::fs::remove_file(&file.path).await;
    }
}

async fn read_text_field(field: &mut Field<'_>, max_bytes: usize) -> Result<String, ShareError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|error| ShareError::bad_request(error.to_string()))?
    {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ShareError::bad_request("Multipart text field is too long"));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| ShareError::bad_request("Multipart text field is invalid"))
}

async fn take_matching_share(store: &LinkShareStore, share_id: Uuid) -> Option<ActiveLinkShare> {
    let mut active = store.0.write().await;
    active
        .as_ref()
        .is_some_and(|share| share.share_id == share_id)
        .then(|| active.take())
        .flatten()
}

struct StagedShareFiles {
    paths: Vec<PathBuf>,
    armed: bool,
}

impl Default for StagedShareFiles {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            armed: true,
        }
    }
}

impl StagedShareFiles {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagedShareFiles {
    fn drop(&mut self) {
        if !self.armed || self.paths.is_empty() {
            return;
        }
        let paths = std::mem::take(&mut self.paths);
        tokio::spawn(async move {
            for path in paths {
                let _ = tokio::fs::remove_file(path).await;
            }
        });
    }
}

struct PendingShareRequestGuard {
    store: LinkShareStore,
    session_id: String,
    request_id: Uuid,
    armed: bool,
}

impl PendingShareRequestGuard {
    fn new(store: LinkShareStore, session_id: String, request_id: Uuid) -> Self {
        Self {
            store,
            session_id,
            request_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    async fn clear(&mut self) {
        self.armed = false;
        clear_pending_request(&self.store, &self.session_id, self.request_id).await;
    }
}

impl Drop for PendingShareRequestGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let store = self.store.clone();
        let session_id = std::mem::take(&mut self.session_id);
        let request_id = self.request_id;
        tokio::spawn(async move {
            clear_pending_request(&store, &session_id, request_id).await;
        });
    }
}

async fn clear_pending_request(store: &LinkShareStore, session_id: &str, request_id: Uuid) {
    let mut store = store.0.write().await;
    let Some(active) = store.as_mut() else {
        return;
    };
    if active.requests.get(session_id).is_some_and(|request| {
        request.request_id == request_id && matches!(request.status, ShareRequestStatus::Pending)
    }) {
        active.requests.remove(session_id);
    }
}

pub(crate) async fn cleanup_stale_share_files(temp_dir: &Path) -> anyhow::Result<()> {
    let mut entries = match tokio::fs::read_dir(temp_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(SHARE_TEMP_PREFIX))
        {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}

fn normalize_pin(pin: String) -> Option<String> {
    let pin = pin.trim();
    (!pin.is_empty()).then(|| pin.to_owned())
}

fn safe_file_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("file")
        .to_owned()
}

fn content_disposition(file_name: &str) -> String {
    let encoded = file_name
        .as_bytes()
        .iter()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (*byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect::<String>();
    format!("attachment; filename=\"download\"; filename*=UTF-8''{encoded}")
}

fn no_store(response: &mut Response) {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
}

struct ShareError {
    status: StatusCode,
    message: String,
}

impl ShareError {
    fn bad_request(message: impl ToString) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn unauthorized(message: impl ToString) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    fn forbidden(message: impl ToString) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    fn not_found(message: impl ToString) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn too_large(message: impl ToString) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, message)
    }

    fn too_many(message: impl ToString) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, message)
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

impl IntoResponse for ShareError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorBody {
            error: String,
        }
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_share() -> ActiveLinkShare {
        ActiveLinkShare {
            share_id: Uuid::new_v4(),
            share_url: "https://localsendy.example/share".to_owned(),
            files: HashMap::new(),
            total_bytes: 0,
            auto_accept: false,
            pin: None,
            requests: HashMap::new(),
            pin_attempts: HashMap::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn selected_share_url_is_normalized_to_the_share_path() {
        let Ok(url) = normalize_share_url("https://localsendy.example/control") else {
            panic!("valid HTTPS URL should be accepted");
        };
        assert_eq!(url, "https://localsendy.example/share");
        assert!(normalize_share_url("file:///tmp/share").is_err());
        assert!(normalize_share_url("https://user@example.com/share").is_err());
    }

    #[test]
    fn download_names_are_encoded_without_header_injection() {
        assert_eq!(
            content_disposition("报告\r\n.txt"),
            "attachment; filename=\"download\"; filename*=UTF-8''%E6%8A%A5%E5%91%8A%0D%0A.txt"
        );
    }

    #[test]
    fn pin_attempts_are_limited_and_reset_after_success() {
        let mut active = empty_share();
        active.pin = Some("123456".to_owned());
        let ip = "192.168.1.2".parse().unwrap();
        assert!(check_pin(&mut active, ip, Some("bad")).is_err());
        assert!(check_pin(&mut active, ip, Some("123456")).is_ok());
        assert!(!active.pin_attempts.contains_key(&ip));
    }

    #[tokio::test]
    async fn pending_guard_only_removes_its_own_request() {
        let store = LinkShareStore::default();
        let session_id = "192.168.1.2".to_owned();
        let first_request_id = Uuid::new_v4();
        let second_request_id = Uuid::new_v4();
        let mut active = empty_share();
        active.requests.insert(
            session_id.clone(),
            ShareRequest {
                request_id: second_request_id,
                ip: session_id.parse().unwrap(),
                user_agent: None,
                status: ShareRequestStatus::Pending,
                decision_tx: None,
                created_at: String::new(),
            },
        );
        *store.0.write().await = Some(active);

        clear_pending_request(&store, &session_id, first_request_id).await;
        assert!(
            store
                .0
                .read()
                .await
                .as_ref()
                .unwrap()
                .requests
                .contains_key(&session_id)
        );

        clear_pending_request(&store, &session_id, second_request_id).await;
        assert!(store.0.read().await.as_ref().unwrap().requests.is_empty());
    }

    #[tokio::test]
    async fn stale_share_id_cannot_stop_the_current_share() {
        let store = LinkShareStore::default();
        let current = empty_share();
        let current_id = current.share_id;
        *store.0.write().await = Some(current);

        assert!(take_matching_share(&store, Uuid::new_v4()).await.is_none());
        assert_eq!(store.0.read().await.as_ref().unwrap().share_id, current_id);

        assert!(take_matching_share(&store, current_id).await.is_some());
        assert!(store.0.read().await.is_none());
    }
}
