use crate::{DeviceIdentity, DeviceInfo, FileId, FileMetadata, ReceivedFile};
use anyhow::Context;
use localsend::http::{
    server::{
        ServerConfigV2, ServerHandle,
        common::save::FileUploadTarget,
        start_with_port,
        v2::{PrepareUploadDecisionV2, ServerEventV2},
    },
    state::ClientInfo,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    sync::{RwLock, mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{debug, warn};
use uuid::Uuid;

pub struct PendingTransfer {
    pub session_id: String,
    pub sender: DeviceInfo,
    pub files: HashMap<FileId, FileMetadata>,
    pub response_tx: oneshot::Sender<bool>,
}

#[derive(Clone, Default)]
pub struct ReceiverState {
    pub pending_transfer: Arc<RwLock<Option<PendingTransfer>>>,
    pub received_files: Arc<RwLock<Vec<ReceivedFile>>>,
    pub destination: Arc<RwLock<PathBuf>>,
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
    auto_accept: bool,
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
    let event_task = tokio::spawn(run_events(event_rx, auto_accept, max_upload_bytes, state));

    Ok(ReceiverHandle {
        server,
        stop_tx: Some(stop_tx),
        event_task,
    })
}

async fn run_events(
    mut events: mpsc::Receiver<ServerEventV2>,
    auto_accept: bool,
    max_upload_bytes: u64,
    state: ReceiverState,
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
                sessions.write().await.insert(
                    session_id.clone(),
                    SessionSender {
                        alias: sender.alias.clone(),
                    },
                );

                if auto_accept {
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
                tokio::spawn(async move {
                    let decision = match response_rx.await {
                        Ok(true) => {
                            PrepareUploadDecisionV2::Accept(files.keys().cloned().collect())
                        }
                        Ok(false) | Err(_) => PrepareUploadDecisionV2::Decline,
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
                let destination = state.destination.read().await.clone();
                if let Err(error) = tokio::fs::create_dir_all(&destination).await {
                    warn!(%session_id, %file_id, %error, "failed to prepare LocalSend destination");
                    continue;
                }
                let temporary_path =
                    destination.join(format!(".localsendy-part-{}", Uuid::new_v4()));
                let (result_tx, result_rx) = oneshot::channel();
                let sender = sessions
                    .read()
                    .await
                    .get(&session_id)
                    .map(|session| session.alias.clone())
                    .unwrap_or_else(|| "Unknown".to_owned());
                let history = state.received_files.clone();
                let completed_tx = state.completed_tx.clone();
                let destination = destination.clone();
                let requested_name = file.file_name.clone();
                let result_path = temporary_path.clone();
                let file_size = file.size;
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
                        }
                        Ok(Err(error)) => {
                            let _ = tokio::fs::remove_file(&result_path).await;
                            warn!(%session_id, %file_id, %error, "LocalSend upload failed")
                        }
                        Err(_) => {
                            let _ = tokio::fs::remove_file(&result_path).await;
                            warn!(%session_id, %file_id, "LocalSend upload result channel closed")
                        }
                    }
                });
                let _ = target_tx.send(FileUploadTarget::Path {
                    path: temporary_path,
                    result_tx,
                    progress_tx: None,
                });
            }
            ServerEventV2::SessionEnd { session_id, .. }
            | ServerEventV2::PrepareUploadAborted { session_id } => {
                sessions.write().await.remove(&session_id);
                let mut pending = state.pending_transfer.write().await;
                if pending
                    .as_ref()
                    .is_some_and(|pending| pending.session_id == session_id)
                {
                    *pending = None;
                }
            }
            ServerEventV2::CancelReceived { .. } => {}
        }
    }
}

#[derive(Clone)]
struct SessionSender {
    alias: String,
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
    use super::{cleanup_partial_files, finalize_upload};

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
}
