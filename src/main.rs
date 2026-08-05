mod api;
mod config;
mod network;
mod state;
mod web;

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{Router, http::HeaderValue};
use config::Config;
use localsend_rs::server::PendingTransfer;
use localsend_rs::{
    DeviceInfo, DeviceType, LocalSendServer, Protocol, generate_tls_certificate, get_device_model,
};
use network::{DiscoveryCommand, NetworkPreferences, run_discovery, run_ipv6_tcp_proxy};
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
    let config = Config::from_env()?;
    tokio::fs::create_dir_all(config.downloads_dir())
        .await
        .context("failed to create downloads directory")?;
    tokio::fs::create_dir_all(config.temp_dir())
        .await
        .context("failed to create temporary upload directory")?;

    let tls_certificate = generate_tls_certificate()?;
    let local_device = DeviceInfo {
        alias: config.alias.clone(),
        version: localsend_rs::PROTOCOL_VERSION.to_owned(),
        device_model: Some(get_device_model()),
        device_type: Some(DeviceType::Server),
        fingerprint: tls_certificate.fingerprint.clone(),
        port: config.localsend_port,
        protocol: Protocol::Https,
        download: false,
        ip: None,
    };

    let devices = Arc::new(RwLock::new(HashMap::<String, SeenDevice>::new()));
    let pending_transfer = Arc::new(AsyncRwLock::new(None));
    let received_files = Arc::new(AsyncRwLock::new(Vec::new()));
    let outgoing_transfers = Arc::new(AsyncRwLock::new(Vec::new()));
    let network_preferences = Arc::new(RwLock::new(NetworkPreferences::load(
        &config.network_config_path(),
        config.network_selection.clone(),
    )?));
    let (scan_tx, scan_rx) = mpsc::channel::<DiscoveryCommand>(4);

    let mut receiver = LocalSendServer::new_with_device(
        local_device.clone(),
        config.downloads_dir(),
        true,
        pending_transfer.clone(),
        received_files.clone(),
    )?;
    receiver.set_tls_certificate(tls_certificate);
    receiver.start(None).await?;

    let ipv6_port = config.localsend_port;
    tokio::spawn(async move {
        if let Err(error) = run_ipv6_tcp_proxy(ipv6_port).await {
            warn!(%error, "LocalSend IPv6 proxy stopped");
        }
    });

    let discovery_device = local_device.clone();
    let discovery_devices = devices.clone();
    let discovery_preferences = network_preferences.clone();
    tokio::spawn(async move {
        if let Err(error) = run_discovery(
            discovery_device,
            discovery_devices,
            discovery_preferences,
            scan_rx,
            config.discovery_interval_seconds,
        )
        .await
        {
            warn!(%error, "LocalSend discovery stopped");
        }
    });

    if config.auto_accept {
        tokio::spawn(run_auto_accept(pending_transfer.clone()));
    }

    let state = AppState {
        config: config.clone(),
        local_device,
        devices,
        pending_transfer,
        received_files,
        outgoing_transfers,
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
        "LocalSend receiver is ready"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    receiver.stop();
    Ok(())
}

async fn run_auto_accept(pending_transfer: Arc<AsyncRwLock<Option<PendingTransfer>>>) {
    let mut interval = tokio::time::interval(Duration::from_millis(200));
    loop {
        interval.tick().await;
        if let Some(transfer) = pending_transfer.write().await.take() {
            let _ = transfer.response_tx.send(true);
        }
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
