mod api;
mod config;
mod network;
mod state;
mod storage_path;
mod web;

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use anyhow::{Context, Result};
use axum::{Router, http::HeaderValue};
use config::Config;
use localsendy_core::{
    DeviceIdentity, DeviceInfo, DeviceType, PROTOCOL_VERSION, PendingTransfer, Protocol,
    ReceivedFile, ReceiverState, start_receiver,
};
use localsendy_storage::{Database, InstanceDefaults, InstanceKey, SettingScope, TransferRecord};
use network::{DiscoveryCommand, NetworkPreferences, run_discovery};
use state::{AppState, SeenDevice};
use tokio::sync::{RwLock as AsyncRwLock, mpsc};
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer,
    set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    install_crypto_provider()?;
    let mut config = Config::from_env()?;

    let database = Database::open(&config.database_path())?;
    let instance_key = InstanceKey::single();
    let instance = database.ensure_instance(InstanceDefaults {
        key: &instance_key,
        alias: &config.alias,
        device_type: device_type_name(config.device_type),
        device_model: config.device_model.as_deref(),
        preferred_port: config.localsend_port,
        identity_path: &config.data_dir.join("identity.pem").display().to_string(),
        download_path: &config.downloads_dir().display().to_string(),
    })?;
    config.alias = instance.alias.clone();
    config.localsend_port = instance.port;

    tokio::fs::create_dir_all(config.downloads_dir())
        .await
        .context("failed to create downloads directory")?;
    tokio::fs::create_dir_all(config.temp_dir())
        .await
        .context("failed to create temporary upload directory")?;

    let download_root = config.downloads_dir();
    let configured_subdirectory = database
        .load_setting::<String>(SettingScope::Instance(&instance_key), "save_subdirectory")?
        .unwrap_or_default();
    let (download_subdirectory, receiver_path) =
        storage_path::resolve_subdirectory(&download_root, &configured_subdirectory).await?;
    tokio::fs::create_dir_all(&receiver_path).await?;
    let download_subdirectory = Arc::new(AsyncRwLock::new(download_subdirectory));
    let receiver_destination = Arc::new(AsyncRwLock::new(receiver_path));

    let identity = Arc::new(DeviceIdentity::load_or_generate(
        &config.data_dir,
        config.alias.clone(),
        config.localsend_port,
    )?);
    let local_device = DeviceInfo {
        alias: config.alias.clone(),
        version: PROTOCOL_VERSION.to_owned(),
        device_model: Some(config.device_model.clone().unwrap_or_else(default_model)),
        device_type: Some(config.device_type),
        fingerprint: identity.material.fingerprint.clone(),
        port: config.localsend_port,
        protocol: Protocol::Https,
        download: false,
        ip: None,
    };

    let devices = Arc::new(RwLock::new(HashMap::<String, SeenDevice>::new()));
    let pending_transfer = Arc::new(AsyncRwLock::new(None::<PendingTransfer>));
    let existing_received = database
        .list_transfers(&instance_key, 500)?
        .into_iter()
        .filter(|transfer| transfer.direction == "incoming" && transfer.status == "completed")
        .map(|transfer| ReceivedFile {
            file_name: transfer.file_name,
            size: transfer.size,
            sender: transfer.peer_alias,
            time: chrono::DateTime::from_timestamp_millis(transfer.created_at_ms)
                .unwrap_or_default()
                .to_rfc3339(),
        })
        .collect::<Vec<_>>();
    let received_files = Arc::new(AsyncRwLock::new(existing_received));
    let (received_tx, mut received_rx) = mpsc::channel::<ReceivedFile>(32);
    let received_database = database.clone();
    let received_instance_id = instance.instance_id.clone();
    tokio::spawn(async move {
        while let Some(received) = received_rx.recv().await {
            let created_at_ms = chrono::DateTime::parse_from_rfc3339(&received.time)
                .map(|time| time.timestamp_millis())
                .unwrap_or_else(|_| chrono::Utc::now().timestamp_millis());
            if let Err(error) = received_database.record_transfer(&TransferRecord {
                id: uuid::Uuid::new_v4().to_string(),
                instance_id: received_instance_id.clone(),
                direction: "incoming".to_owned(),
                peer_alias: received.sender,
                file_name: received.file_name,
                size: received.size,
                status: "completed".to_owned(),
                created_at_ms,
                error: None,
            }) {
                warn!(%error, "failed to persist incoming transfer");
            }
        }
    });
    let receiver = start_receiver(
        &identity,
        local_device.clone(),
        config.auto_accept,
        config.max_upload_bytes,
        ReceiverState {
            pending_transfer: pending_transfer.clone(),
            received_files: received_files.clone(),
            destination: receiver_destination.clone(),
            completed_tx: Some(received_tx),
        },
    )
    .await
    .context("failed to start official LocalSend Rust server")?;

    let network_preferences = Arc::new(RwLock::new(NetworkPreferences::load(
        &config.network_config_path(),
        config.network_selection.clone(),
    )?));
    let (scan_tx, scan_rx) = mpsc::channel::<DiscoveryCommand>(4);
    let discovery_devices_info = Arc::new(RwLock::new(vec![local_device.clone()]));
    let discovery_devices = devices.clone();
    let discovery_preferences = network_preferences.clone();
    let discovery_interval = config.discovery_interval_seconds;
    tokio::spawn(async move {
        if let Err(error) = run_discovery(
            discovery_devices_info,
            discovery_devices,
            discovery_preferences,
            scan_rx,
            discovery_interval,
        )
        .await
        {
            warn!(%error, "LocalSend discovery stopped");
        }
    });

    let state = AppState {
        config: config.clone(),
        database,
        instance_key,
        identity,
        local_device,
        devices,
        pending_transfer,
        received_files,
        outgoing_transfers: Arc::new(AsyncRwLock::new(Vec::new())),
        download_root,
        download_subdirectory,
        receiver_destination,
        scan_tx,
        network_preferences,
        started_at: Instant::now(),
    };

    let app = Router::new()
        .nest("/api/v1", api::router(state))
        .fallback(web::static_handler)
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(CompressionLayer::new())
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(config.web_bind)
        .await
        .with_context(|| format!("failed to bind web UI on {}", config.web_bind))?;
    info!(address = %config.web_bind, "Localsendy web UI is ready");
    info!(
        port = config.localsend_port,
        alias = config.alias,
        "Official LocalSend Rust server is ready"
    );

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    receiver.stop().await;
    result?;
    Ok(())
}

fn default_model() -> String {
    std::env::consts::OS.to_owned()
}

fn device_type_name(device_type: DeviceType) -> &'static str {
    match device_type {
        DeviceType::Mobile => "mobile",
        DeviceType::Desktop => "desktop",
        DeviceType::Web => "web",
        DeviceType::Headless => "headless",
        DeviceType::Server => "server",
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("localsendy=info,tower_http=info")),
        )
        .compact()
        .init();
}

fn install_crypto_provider() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a Rustls crypto provider was already installed"))
}

#[cfg(test)]
mod tests {
    use super::install_crypto_provider;

    #[test]
    fn installs_ring_crypto_provider() {
        let _ = install_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
