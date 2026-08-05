use crate::{DeviceIdentity, DeviceInfo, FileId, FileMetadata, ReceivedFile};
use anyhow::Context;
use localsend::http::{
    server::{
        ServerConfigV2, ServerHandle,
        common::save::FileUploadTarget,
        start_with_port,
        v2::{PrepareUploadDecisionV2, ServerEventV2, SessionEndReasonV2},
    },
    state::ClientInfo,
};
use serde::{Serialize, Serializer};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{RwLock, mpsc, oneshot, watch},
    task::JoinHandle,
    time::Instant,
};
use tracing::{debug, warn};
use uuid::Uuid;

const MAX_INCOMING_FILES: usize = 100;
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub struct PendingTransfer {
    pub session_id: String,
    pub sender: DeviceInfo,
    pub files: HashMap<FileId, FileMetadata>,
    pub response_tx: oneshot::Sender<bool>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IncomingTransferStatus {
    Waiting,
    Receiving,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingTransfer {
    pub id: String,
    pub session_id: String,
    pub file_id: String,
    pub sender_alias: String,
    pub file_name: String,
    pub total_bytes: u64,
    #[serde(serialize_with = "serialize_atomic_u64")]
    pub transferred_bytes: Arc<AtomicU64>,
    pub status: IncomingTransferStatus,
    pub created_at: String,
    pub error: Option<String>,
}

fn serialize_atomic_u64<S>(value: &Arc<AtomicU64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u64(value.load(Ordering::Relaxed))
}

#[derive(Clone, Default)]
pub struct ReceiverState {
    pub pending_transfer: Arc<RwLock<Option<PendingTransfer>>>,
    pub received_files: Arc<RwLock<Vec<ReceivedFile>>>,
    pub incoming_transfers: Arc<RwLock<Vec<IncomingTransfer>>>,
    pub destination: Arc<RwLock<PathBuf>>,
    pub auto_accept: bool,
    pub completed_tx: Option<mpsc::Sender<ReceivedFile>>,
}

pub struct ReceiverHandle {
    server: Arc<ServerHandle>,
    stop_tx: Option<oneshot::Sender<()>>,
    event_task: JoinHandle<()>,
}

impl ReceiverHandle {
    pub async fn stop(mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        self.server.wait_stopped().await;
        self.event_task.abort();
        let _ = self.event_task.await;
    }
}

pub async fn start_receiver(
    identity: &DeviceIdentity,
    device: DeviceInfo,
    max_upload_bytes: u64,
    state: ReceiverState,
) -> anyhow::Result<ReceiverHandle> {
    let destination = state.destination.read().await.clone();
    tokio::fs::create_dir_all(&destination)
        .await
        .with_context(|| format!("failed to create {}", destination.display()))?;
    cleanup_partial_files(&destination).await?;

    let (event_tx, event_rx) = mpsc::channel::<ServerEventV2>(32);
    let (stop_tx, stop_rx) = oneshot::channel();
    let server = start_with_port(
        device.port,
        Some(identity.tls_config()),
        ClientInfo {
            alias: device.alias.clone(),
            version: device.version.clone(),
            device_model: device.device_model.clone(),
            device_type: device.device_type.map(Into::into),
            token: device.fingerprint.clone(),
        },
        None,
        Some(ServerConfigV2 {
            pin: None,
            verify_checksums: true,
            event_tx,
        }),
        None,
        stop_rx,
    )
    .await?;
    let server = Arc::new(server);
    let event_task = tokio::spawn(run_events(
        event_rx,
        max_upload_bytes,
        state,
        Arc::downgrade(&server),
    ));

    Ok(ReceiverHandle {
        server,
        stop_tx: Some(stop_tx),
        event_task,
    })
}

async fn run_events(
    mut events: mpsc::Receiver<ServerEventV2>,
    max_upload_bytes: u64,
    state: ReceiverState,
    server: Weak<ServerHandle>,
) {
    let sessions = Arc::new(RwLock::new(HashMap::<String, SessionSender>::new()));
    while let Some(event) = events.recv().await {
        match event {
            ServerEventV2::Register { ip, info } => {
                debug!(alias = info.alias, address = %ip, "LocalSend peer registered");
            }
            ServerEventV2::PrepareUpload {
                session_id,
                ip,
                info,
                files,
                decision_tx,
                ..
            } => {
                let total_bytes = files.values().map(|file| file.size).sum::<u64>();
                if total_bytes > max_upload_bytes {
                    let _ = decision_tx.send(PrepareUploadDecisionV2::Decline);
                    warn!(%session_id, total_bytes, max_upload_bytes, "declined oversized LocalSend transfer");
                    continue;
                }
                if files.len() > MAX_INCOMING_FILES {
                    let _ = decision_tx.send(PrepareUploadDecisionV2::Decline);
                    warn!(
                        %session_id,
                        file_count = files.len(),
                        max_file_count = MAX_INCOMING_FILES,
                        "declined LocalSend transfer with too many files"
                    );
                    continue;
                }

                let sender = DeviceInfo {
                    alias: info.alias.clone(),
                    version: info.version.clone(),
                    device_model: info.device_model.clone(),
                    device_type: info.device_type.clone().map(Into::into),
                    fingerprint: info.fingerprint.clone(),
                    port: info.port,
                    protocol: info.protocol.into(),
                    download: info.download,
                    ip: Some(ip.to_string()),
                };
                let web_files = files
                    .values()
                    .cloned()
                    .map(FileMetadata::from)
                    .map(|file| (file.id.clone(), file))
                    .collect::<HashMap<_, _>>();
                {
                    let mut incoming = state.incoming_transfers.write().await;
                    if incoming.len() + files.len() > 100 {
                        let remove_count = incoming.len() + files.len() - 100;
                        let remove_count = remove_count.min(incoming.len());
                        incoming.drain(0..remove_count);
                    }
                    incoming.extend(files.iter().map(|(file_id, file)| IncomingTransfer {
                        id: format!("{session_id}:{file_id}"),
                        session_id: session_id.clone(),
                        file_id: file_id.clone(),
                        sender_alias: sender.alias.clone(),
                        file_name: file.file_name.clone(),
                        total_bytes: file.size,
                        transferred_bytes: Arc::new(AtomicU64::new(0)),
                        status: IncomingTransferStatus::Waiting,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        error: None,
                    }));
                }
                let (activity_tx, activity_rx) = watch::channel(Instant::now());
                let (stop_tx, stop_rx) = watch::channel(false);
                sessions.write().await.insert(
                    session_id.clone(),
                    SessionSender {
                        alias: sender.alias.clone(),
                        activity_tx,
                        stop_tx,
                    },
                );
                spawn_session_watchdog(
                    session_id.clone(),
                    activity_rx,
                    stop_rx,
                    Arc::downgrade(&sessions),
                    state.pending_transfer.clone(),
                    state.incoming_transfers.clone(),
                    server.clone(),
                );

                if state.auto_accept {
                    let ids = files.keys().cloned().collect::<HashSet<_>>();
                    let _ = decision_tx.send(PrepareUploadDecisionV2::Accept(ids));
                    continue;
                }

                let (response_tx, response_rx) = oneshot::channel::<bool>();
                *state.pending_transfer.write().await = Some(PendingTransfer {
                    session_id: session_id.clone(),
                    sender,
                    files: web_files,
                    response_tx,
                });
                let incoming_transfers = state.incoming_transfers.clone();
                let pending_session_id = session_id.clone();
                tokio::spawn(async move {
                    let decision = match response_rx.await {
                        Ok(true) => {
                            PrepareUploadDecisionV2::Accept(files.keys().cloned().collect())
                        }
                        Ok(false) => {
                            fail_incomplete_session(
                                &incoming_transfers,
                                &pending_session_id,
                                "Transfer declined",
                            )
                            .await;
                            PrepareUploadDecisionV2::Decline
                        }
                        Err(_) => {
                            fail_incomplete_session(
                                &incoming_transfers,
                                &pending_session_id,
                                "Transfer request expired",
                            )
                            .await;
                            PrepareUploadDecisionV2::Decline
                        }
                    };
                    let _ = decision_tx.send(decision);
                });
            }
            ServerEventV2::FileUpload {
                session_id,
                file_id,
                file,
                target_tx,
            } => {
                let session = sessions.read().await.get(&session_id).cloned();
                if let Some(session) = &session {
                    session.touch();
                }
                let destination = state.destination.read().await.clone();
                if let Err(error) = tokio::fs::create_dir_all(&destination).await {
                    update_incoming_result(
                        &state.incoming_transfers,
                        &session_id,
                        &file_id,
                        IncomingTransferStatus::Failed,
                        Some(error.to_string()),
                    )
                    .await;
                    warn!(%session_id, %file_id, %error, "failed to prepare LocalSend destination");
                    continue;
                }
                let temporary_path =
                    destination.join(format!(".localsendy-part-{}", Uuid::new_v4()));
                let (result_tx, result_rx) = oneshot::channel::<Result<(), String>>();
                let sender = session
                    .as_ref()
                    .map(|session| session.alias.clone())
                    .unwrap_or_else(|| "Unknown".to_owned());
                let history = state.received_files.clone();
                let completed_tx = state.completed_tx.clone();
                let destination = destination.clone();
                let requested_name = file.file_name.clone();
                let result_path = temporary_path.clone();
                let file_size = file.size;
                let progress = {
                    let mut incoming = state.incoming_transfers.write().await;
                    incoming
                        .iter_mut()
                        .find(|transfer| {
                            transfer.session_id == session_id && transfer.file_id == file_id
                        })
                        .map(|transfer| {
                            transfer.status = IncomingTransferStatus::Receiving;
                            transfer.transferred_bytes.clone()
                        })
                        .unwrap_or_else(|| Arc::new(AtomicU64::new(0)))
                };
                let (progress_tx, mut progress_rx) = mpsc::channel::<u64>(16);
                let progress_counter = progress.clone();
                let progress_activity = session.map(|session| session.activity_tx);
                tokio::spawn(async move {
                    while let Some(written) = progress_rx.recv().await {
                        progress_counter.store(written.min(file_size), Ordering::Relaxed);
                        if let Some(activity) = &progress_activity {
                            activity.send_replace(Instant::now());
                        }
                    }
                });
                let incoming_transfers = state.incoming_transfers.clone();
                let progress_session_id = session_id.clone();
                let progress_file_id = file_id.clone();
                tokio::spawn(async move {
                    match result_rx.await {
                        Ok(Ok(())) => {
                            let final_path = match finalize_upload(
                                &result_path,
                                &destination,
                                &requested_name,
                            )
                            .await
                            {
                                Ok(path) => path,
                                Err(error) => {
                                    let _ = tokio::fs::remove_file(&result_path).await;
                                    update_incoming_result(
                                        &incoming_transfers,
                                        &progress_session_id,
                                        &progress_file_id,
                                        IncomingTransferStatus::Failed,
                                        Some(error.to_string()),
                                    )
                                    .await;
                                    warn!(%session_id, %file_id, %error, "failed to finalize LocalSend upload");
                                    return;
                                }
                            };
                            let file_name = final_path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or(&requested_name)
                                .to_owned();
                            let received = ReceivedFile {
                                file_name,
                                size: file_size,
                                sender,
                                time: chrono::Utc::now().to_rfc3339(),
                            };
                            history.write().await.push(received.clone());
                            if let Some(completed_tx) = &completed_tx {
                                let _ = completed_tx.send(received).await;
                            }
                            progress.store(file_size, Ordering::Relaxed);
                            update_incoming_result(
                                &incoming_transfers,
                                &progress_session_id,
                                &progress_file_id,
                                IncomingTransferStatus::Completed,
                                None,
                            )
                            .await;
                        }
                        Ok(Err(error)) => {
                            let _ = tokio::fs::remove_file(&result_path).await;
                            update_incoming_result(
                                &incoming_transfers,
                                &progress_session_id,
                                &progress_file_id,
                                IncomingTransferStatus::Failed,
                                Some(error.clone()),
                            )
                            .await;
                            warn!(%session_id, %file_id, %error, "LocalSend upload failed")
                        }
                        Err(_) => {
                            let _ = tokio::fs::remove_file(&result_path).await;
                            update_incoming_result(
                                &incoming_transfers,
                                &progress_session_id,
                                &progress_file_id,
                                IncomingTransferStatus::Failed,
                                Some("Upload result channel closed".to_owned()),
                            )
                            .await;
                            warn!(%session_id, %file_id, "LocalSend upload result channel closed")
                        }
                    }
                });
                let _ = target_tx.send(FileUploadTarget::Path {
                    path: temporary_path,
                    result_tx,
                    progress_tx: Some(progress_tx),
                });
            }
            ServerEventV2::SessionEnd { session_id, reason } => {
                if let Some(session) = sessions.write().await.remove(&session_id) {
                    session.stop();
                }
                let mut pending = state.pending_transfer.write().await;
                if pending
                    .as_ref()
                    .is_some_and(|pending| pending.session_id == session_id)
                {
                    *pending = None;
                }
                drop(pending);
                if reason == SessionEndReasonV2::Cancelled {
                    fail_incomplete_session(
                        &state.incoming_transfers,
                        &session_id,
                        "Transfer cancelled by sender",
                    )
                    .await;
                }
            }
            ServerEventV2::PrepareUploadAborted { session_id } => {
                if let Some(session) = sessions.write().await.remove(&session_id) {
                    session.stop();
                }
                let mut pending = state.pending_transfer.write().await;
                if pending
                    .as_ref()
                    .is_some_and(|pending| pending.session_id == session_id)
                {
                    *pending = None;
                }
                drop(pending);
                fail_incomplete_session(
                    &state.incoming_transfers,
                    &session_id,
                    "Transfer request aborted",
                )
                .await;
            }
            ServerEventV2::CancelReceived { .. } => {}
        }
    }
}

async fn update_incoming_result(
    transfers: &RwLock<Vec<IncomingTransfer>>,
    session_id: &str,
    file_id: &str,
    status: IncomingTransferStatus,
    error: Option<String>,
) {
    if let Some(transfer) = transfers
        .write()
        .await
        .iter_mut()
        .find(|transfer| transfer.session_id == session_id && transfer.file_id == file_id)
    {
        transfer.status = status;
        transfer.error = error;
    }
}

async fn fail_incomplete_session(
    transfers: &RwLock<Vec<IncomingTransfer>>,
    session_id: &str,
    error: &str,
) {
    for transfer in transfers
        .write()
        .await
        .iter_mut()
        .filter(|transfer| transfer.session_id == session_id)
    {
        if matches!(
            transfer.status,
            IncomingTransferStatus::Waiting | IncomingTransferStatus::Receiving
        ) {
            transfer.status = IncomingTransferStatus::Failed;
            transfer.error = Some(error.to_owned());
        }
    }
}

#[derive(Clone)]
struct SessionSender {
    alias: String,
    activity_tx: watch::Sender<Instant>,
    stop_tx: watch::Sender<bool>,
}

impl SessionSender {
    fn touch(&self) {
        self.activity_tx.send_replace(Instant::now());
    }

    fn stop(&self) {
        self.stop_tx.send_replace(true);
    }
}

fn spawn_session_watchdog(
    session_id: String,
    activity_rx: watch::Receiver<Instant>,
    stop_rx: watch::Receiver<bool>,
    sessions: Weak<RwLock<HashMap<String, SessionSender>>>,
    pending_transfer: Arc<RwLock<Option<PendingTransfer>>>,
    incoming_transfers: Arc<RwLock<Vec<IncomingTransfer>>>,
    server: Weak<ServerHandle>,
) {
    tokio::spawn(async move {
        if !wait_for_session_idle(activity_rx, stop_rx, SESSION_IDLE_TIMEOUT).await {
            return;
        }

        if let Some(server) = server.upgrade() {
            server.cancel_v2_session(&session_id).await;
        }
        if let Some(sessions) = sessions.upgrade() {
            sessions.write().await.remove(&session_id);
        }
        let mut pending = pending_transfer.write().await;
        if pending
            .as_ref()
            .is_some_and(|pending| pending.session_id == session_id)
        {
            *pending = None;
        }
        drop(pending);
        fail_incomplete_session(
            &incoming_transfers,
            &session_id,
            "Transfer timed out after 120 seconds without activity",
        )
        .await;
        warn!(%session_id, "cancelled idle LocalSend upload session");
    });
}

async fn wait_for_session_idle(
    mut activity_rx: watch::Receiver<Instant>,
    mut stop_rx: watch::Receiver<bool>,
    timeout: Duration,
) -> bool {
    loop {
        let deadline = *activity_rx.borrow() + timeout;
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                if Instant::now().duration_since(*activity_rx.borrow()) >= timeout {
                    return true;
                }
            }
            changed = activity_rx.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    return false;
                }
            }
        }
    }
}

async fn cleanup_partial_files(destination: &Path) -> anyhow::Result<()> {
    let mut entries = tokio::fs::read_dir(destination)
        .await
        .with_context(|| format!("failed to read {}", destination.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(".localsendy-part-") {
            tokio::fs::remove_file(entry.path())
                .await
                .with_context(|| {
                    format!("failed to remove stale upload {}", entry.path().display())
                })?;
        }
    }
    Ok(())
}

async fn finalize_upload(
    temporary_path: &Path,
    destination: &Path,
    requested_name: &str,
) -> anyhow::Result<PathBuf> {
    for index in 0_u32.. {
        let final_path = numbered_path(destination, requested_name, index);
        match tokio::fs::hard_link(temporary_path, &final_path).await {
            Ok(()) => {
                tokio::fs::remove_file(temporary_path).await?;
                return Ok(final_path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!()
}

fn numbered_path(destination: &Path, requested_name: &str, index: u32) -> PathBuf {
    let safe_name = Path::new(requested_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("file");
    if index == 0 {
        return destination.join(safe_name);
    }

    let path = Path::new(safe_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let extension = path.extension().and_then(|value| value.to_str());
    let name = match extension {
        Some(extension) => format!("{stem} ({index}).{extension}"),
        None => format!("{stem} ({index})"),
    };
    destination.join(name)
}

#[cfg(test)]
mod tests {
    use super::{
        IncomingTransfer, IncomingTransferStatus, cleanup_partial_files, fail_incomplete_session,
        finalize_upload, wait_for_session_idle,
    };
    use std::sync::{Arc, atomic::AtomicU64};
    use std::time::Duration;
    use tokio::{
        sync::{RwLock, watch},
        time::Instant,
    };

    #[tokio::test]
    async fn finalizes_without_overwriting_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let temporary = directory.path().join(".localsendy-part-test");
        tokio::fs::write(directory.path().join("report.txt"), b"existing")
            .await
            .unwrap();
        tokio::fs::write(&temporary, b"received").await.unwrap();

        let final_path = finalize_upload(&temporary, directory.path(), "report.txt")
            .await
            .unwrap();

        assert_eq!(final_path.file_name().unwrap(), "report (1).txt");
        assert_eq!(tokio::fs::read(final_path).await.unwrap(), b"received");
        assert!(!temporary.exists());
    }

    #[tokio::test]
    async fn removes_stale_partial_files_only() {
        let directory = tempfile::tempdir().unwrap();
        let partial = directory.path().join(".localsendy-part-stale");
        let complete = directory.path().join("complete.txt");
        tokio::fs::write(&partial, b"partial").await.unwrap();
        tokio::fs::write(&complete, b"complete").await.unwrap();

        cleanup_partial_files(directory.path()).await.unwrap();

        assert!(!partial.exists());
        assert!(complete.exists());
    }

    #[tokio::test]
    async fn fails_only_incomplete_files_when_a_session_ends() {
        let transfers = RwLock::new(vec![
            IncomingTransfer {
                id: "session:waiting".to_owned(),
                session_id: "session".to_owned(),
                file_id: "waiting".to_owned(),
                sender_alias: "Phone".to_owned(),
                file_name: "waiting.txt".to_owned(),
                total_bytes: 10,
                transferred_bytes: Arc::new(AtomicU64::new(0)),
                status: IncomingTransferStatus::Waiting,
                created_at: "2026-08-06T00:00:00Z".to_owned(),
                error: None,
            },
            IncomingTransfer {
                id: "session:completed".to_owned(),
                session_id: "session".to_owned(),
                file_id: "completed".to_owned(),
                sender_alias: "Phone".to_owned(),
                file_name: "completed.txt".to_owned(),
                total_bytes: 10,
                transferred_bytes: Arc::new(AtomicU64::new(10)),
                status: IncomingTransferStatus::Completed,
                created_at: "2026-08-06T00:00:00Z".to_owned(),
                error: None,
            },
        ]);

        fail_incomplete_session(&transfers, "session", "Transfer cancelled").await;

        let transfers = transfers.read().await;
        assert!(matches!(
            transfers[0].status,
            IncomingTransferStatus::Failed
        ));
        assert_eq!(transfers[0].error.as_deref(), Some("Transfer cancelled"));
        assert!(matches!(
            transfers[1].status,
            IncomingTransferStatus::Completed
        ));
        assert!(transfers[1].error.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn session_idle_timeout_resets_on_activity_and_stops_on_completion() {
        let (activity_tx, activity_rx) = watch::channel(Instant::now());
        let (stop_tx, stop_rx) = watch::channel(false);
        let timeout = Duration::from_secs(10);
        let task = tokio::spawn(wait_for_session_idle(activity_rx, stop_rx, timeout));

        tokio::time::advance(Duration::from_secs(9)).await;
        activity_tx.send_replace(Instant::now());
        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(!task.is_finished());

        stop_tx.send_replace(true);
        assert!(!task.await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn session_idle_timeout_expires_without_activity() {
        let (_activity_tx, activity_rx) = watch::channel(Instant::now());
        let (_stop_tx, stop_rx) = watch::channel(false);
        let task = tokio::spawn(wait_for_session_idle(
            activity_rx,
            stop_rx,
            Duration::from_secs(10),
        ));

        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(task.await.unwrap());
    }
}
