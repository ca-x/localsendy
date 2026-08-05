use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    path::Path,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use if_addrs::{IfAddr, Interface, get_if_addrs};
use localsend_rs::{
    AnnouncementMessage, DEFAULT_MULTICAST_ADDRESS, DEFAULT_MULTICAST_PORT, DeviceInfo,
};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol as SocketProtocol, Socket, Type};
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream, UdpSocket},
    sync::mpsc,
    task::JoinHandle,
};
use tracing::{debug, info, warn};

use crate::state::SeenDevice;

const DEFAULT_MULTICAST_GROUP_V6: Ipv6Addr = Ipv6Addr::new(0xff12, 0, 0, 0, 0, 0, 0xfd3a, 0xe420);
const INTERFACE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    All,
    Selected,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NetworkSelection {
    pub mode: NetworkMode,
    #[serde(default)]
    pub interfaces: BTreeSet<String>,
}

impl NetworkSelection {
    pub fn all() -> Self {
        Self {
            mode: NetworkMode::All,
            interfaces: BTreeSet::new(),
        }
    }

    pub fn selected(interfaces: BTreeSet<String>) -> Self {
        Self {
            mode: NetworkMode::Selected,
            interfaces,
        }
    }

    fn includes(&self, name: &str) -> bool {
        self.mode == NetworkMode::All || self.interfaces.contains(name)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NetworkPreferences {
    #[serde(flatten)]
    pub selection: NetworkSelection,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

impl NetworkPreferences {
    pub fn new(selection: NetworkSelection) -> Self {
        Self {
            selection,
            labels: BTreeMap::new(),
        }
    }

    pub fn load(path: &Path, fallback: NetworkSelection) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to parse {}", path.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::new(fallback)),
            Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    fn includes(&self, name: &str) -> bool {
        self.selection.includes(name)
    }
}

pub async fn save_preferences(path: &Path, preferences: &NetworkPreferences) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(preferences)?;
    tokio::fs::write(&temporary, bytes)
        .await
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    tokio::fs::rename(&temporary, path)
        .await
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InterfaceKind {
    Ethernet,
    Wifi,
    Bridge,
    Tunnel,
    Virtual,
    Other,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub label: Option<String>,
    pub kind: InterfaceKind,
    pub ipv4_addresses: Vec<String>,
    pub ipv6_addresses: Vec<String>,
    pub ipv4_discovery: bool,
    pub ipv6_discovery: bool,
    pub discovery_capable: bool,
    pub point_to_point: bool,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSettings {
    pub mode: NetworkMode,
    pub selected_interfaces: Vec<String>,
    pub active_discovery_interfaces: Vec<String>,
    pub interfaces: Vec<NetworkInterfaceInfo>,
}

#[derive(Clone, Debug)]
struct InterfaceRecord {
    name: String,
    index: Option<u32>,
    kind: InterfaceKind,
    ipv4: Vec<(Ipv4Addr, u8)>,
    ipv6: Vec<(Ipv6Addr, u8)>,
    point_to_point: bool,
}

impl InterfaceRecord {
    fn ipv4_discovery(&self) -> bool {
        !self.point_to_point
            && self
                .ipv4
                .iter()
                .any(|(address, _)| !address.is_link_local())
    }

    fn ipv6_discovery(&self) -> bool {
        self.index.is_some() && !self.ipv6.is_empty()
    }

    fn has_usable_address(&self) -> bool {
        !self.ipv4.is_empty()
            || self
                .ipv6
                .iter()
                .any(|(address, _)| !address.is_unicast_link_local())
    }
}

pub fn network_settings(preferences: &NetworkPreferences) -> io::Result<NetworkSettings> {
    let records = interface_records(get_if_addrs()?);
    let mut active_discovery_interfaces = Vec::new();
    let interfaces = records
        .into_iter()
        .map(|record| {
            let ipv4_discovery = record.ipv4_discovery();
            let ipv6_discovery = record.ipv6_discovery();
            let discovery_capable = ipv4_discovery || ipv6_discovery;
            let selected = discovery_capable && preferences.includes(&record.name);
            if selected {
                active_discovery_interfaces.push(record.name.clone());
            }
            NetworkInterfaceInfo {
                label: preferences.labels.get(&record.name).cloned(),
                name: record.name,
                kind: record.kind,
                ipv4_addresses: record
                    .ipv4
                    .iter()
                    .map(|(address, prefix)| format!("{address}/{prefix}"))
                    .collect(),
                ipv6_addresses: record
                    .ipv6
                    .iter()
                    .map(|(address, prefix)| format!("{address}/{prefix}"))
                    .collect(),
                ipv4_discovery,
                ipv6_discovery,
                discovery_capable,
                point_to_point: record.point_to_point,
                selected,
            }
        })
        .collect();
    active_discovery_interfaces.sort();
    active_discovery_interfaces.dedup();

    Ok(NetworkSettings {
        mode: preferences.selection.mode,
        selected_interfaces: preferences.selection.interfaces.iter().cloned().collect(),
        active_discovery_interfaces,
        interfaces,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DiscoveryEndpoint {
    V4 { name: String, address: Ipv4Addr },
    V6 { name: String, index: u32 },
}

impl DiscoveryEndpoint {
    fn name(&self) -> &str {
        match self {
            Self::V4 { name, .. } | Self::V6 { name, .. } => name,
        }
    }

    fn description(&self) -> String {
        match self {
            Self::V4 { name, address } => format!("{name} ({address})"),
            Self::V6 { name, index } => format!("{name} (IPv6, if-index {index})"),
        }
    }

    fn is_ipv6(&self) -> bool {
        matches!(self, Self::V6 { .. })
    }
}

fn selected_endpoints(preferences: &NetworkPreferences) -> io::Result<Vec<DiscoveryEndpoint>> {
    let mut endpoints = Vec::new();
    for record in interface_records(get_if_addrs()?) {
        if !preferences.includes(&record.name) {
            continue;
        }
        if record.ipv4_discovery() {
            endpoints.extend(
                record
                    .ipv4
                    .iter()
                    .filter(|(address, _)| !address.is_link_local())
                    .map(|(address, _)| DiscoveryEndpoint::V4 {
                        name: record.name.clone(),
                        address: *address,
                    }),
            );
        }
        if record.ipv6_discovery()
            && let Some(index) = record.index
        {
            endpoints.push(DiscoveryEndpoint::V6 {
                name: record.name,
                index,
            });
        }
    }
    endpoints.sort_by_key(DiscoveryEndpoint::description);
    endpoints.dedup();
    Ok(endpoints)
}

fn interface_records(interfaces: Vec<Interface>) -> Vec<InterfaceRecord> {
    let mut records = BTreeMap::<String, InterfaceRecord>::new();
    for interface in interfaces {
        if interface.is_loopback() || !interface.is_oper_up() {
            continue;
        }
        let record = records
            .entry(interface.name.clone())
            .or_insert_with(|| InterfaceRecord {
                name: interface.name.clone(),
                index: interface.index,
                kind: interface_kind(&interface.name),
                ipv4: Vec::new(),
                ipv6: Vec::new(),
                point_to_point: interface.is_p2p(),
            });
        record.index = record.index.or(interface.index);
        record.point_to_point |= interface.is_p2p();
        match interface.addr {
            IfAddr::V4(address) if !address.ip.is_loopback() && !address.ip.is_unspecified() => {
                record.ipv4.push((address.ip, address.prefixlen));
            }
            IfAddr::V6(address) if !address.ip.is_loopback() && !address.ip.is_unspecified() => {
                record.ipv6.push((address.ip, address.prefixlen));
            }
            _ => {}
        }
    }

    records
        .into_values()
        .filter(InterfaceRecord::has_usable_address)
        .map(|mut record| {
            record.ipv4.sort();
            record.ipv4.dedup();
            record.ipv6.sort();
            record.ipv6.dedup();
            record
        })
        .collect()
}

fn interface_kind(name: &str) -> InterfaceKind {
    let name = name.to_ascii_lowercase();
    if name.starts_with("wl") || name.starts_with("wifi") {
        InterfaceKind::Wifi
    } else if name.starts_with("en") || name.starts_with("eth") {
        InterfaceKind::Ethernet
    } else if name.starts_with("br")
        || name.starts_with("docker")
        || name.starts_with("virbr")
        || name.starts_with("lzc-br")
    {
        InterfaceKind::Bridge
    } else if name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("wg")
        || name.starts_with("tailscale")
        || name.starts_with("heiyu")
    {
        InterfaceKind::Tunnel
    } else if name.starts_with("veth") {
        InterfaceKind::Virtual
    } else {
        InterfaceKind::Other
    }
}

#[derive(Debug)]
pub enum DiscoveryCommand {
    Announce,
    Reconfigure,
}

#[derive(Clone)]
struct BoundDiscoverySocket {
    endpoint: DiscoveryEndpoint,
    target: SocketAddr,
    socket: Arc<UdpSocket>,
}

struct ReceivedDatagram {
    endpoint: DiscoveryEndpoint,
    source: SocketAddr,
    payload: Vec<u8>,
}

pub async fn run_discovery(
    local_device: DeviceInfo,
    devices: Arc<RwLock<HashMap<String, SeenDevice>>>,
    preferences: Arc<RwLock<NetworkPreferences>>,
    mut commands: mpsc::Receiver<DiscoveryCommand>,
    interval_seconds: u64,
) -> Result<()> {
    let multicast_v4 = DEFAULT_MULTICAST_ADDRESS
        .parse::<Ipv4Addr>()
        .context("LocalSend multicast address must be IPv4")?;
    let (datagram_tx, mut datagram_rx) = mpsc::channel::<ReceivedDatagram>(128);
    let mut endpoints = current_selected_endpoints(&preferences)?;
    let mut sockets = bind_discovery_sockets(multicast_v4, &endpoints);
    let mut receive_tasks = spawn_receive_tasks(&sockets, datagram_tx.clone());
    log_active_interfaces(&sockets);

    let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut interface_refresh = tokio::time::interval(INTERFACE_REFRESH_INTERVAL);
    interface_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            datagram = datagram_rx.recv() => {
                let Some(datagram) = datagram else { break };
                if let Some((should_respond, source_interface)) = handle_announcement(
                    &datagram.payload,
                    datagram.source,
                    datagram.endpoint.name(),
                    &local_device,
                    &devices,
                )
                    && should_respond
                    && let Some(response_socket) = sockets
                        .iter()
                        .find(|socket| {
                            socket.endpoint.name() == source_interface
                                && socket.endpoint.is_ipv6() == datagram.source.is_ipv6()
                        })
                        .cloned()
                {
                    let response_device = local_device.clone();
                    tokio::spawn(async move {
                        send_announcement(&response_device, &[response_socket], false).await;
                    });
                }
            }
            _ = interval.tick() => {
                refresh_sockets_if_needed(
                    &preferences,
                    multicast_v4,
                    &mut endpoints,
                    &mut sockets,
                    &mut receive_tasks,
                    &datagram_tx,
                )?;
                send_announcement(&local_device, &sockets, true).await;
            }
            _ = interface_refresh.tick() => {
                if refresh_sockets_if_needed(
                    &preferences,
                    multicast_v4,
                    &mut endpoints,
                    &mut sockets,
                    &mut receive_tasks,
                    &datagram_tx,
                )? {
                    send_announcement(&local_device, &sockets, true).await;
                }
            }
            command = commands.recv() => {
                match command {
                    Some(DiscoveryCommand::Announce) => {
                        refresh_sockets_if_needed(
                            &preferences,
                            multicast_v4,
                            &mut endpoints,
                            &mut sockets,
                            &mut receive_tasks,
                            &datagram_tx,
                        )?;
                        send_announcement(&local_device, &sockets, true).await;
                    }
                    Some(DiscoveryCommand::Reconfigure) => {
                        endpoints = current_selected_endpoints(&preferences)?;
                        replace_sockets(
                            multicast_v4,
                            &endpoints,
                            &mut sockets,
                            &mut receive_tasks,
                            &datagram_tx,
                        );
                        send_announcement(&local_device, &sockets, true).await;
                    }
                    None => break,
                }
            }
        }
    }

    for task in receive_tasks {
        task.abort();
    }
    Ok(())
}

fn refresh_sockets_if_needed(
    preferences: &Arc<RwLock<NetworkPreferences>>,
    multicast_v4: Ipv4Addr,
    endpoints: &mut Vec<DiscoveryEndpoint>,
    sockets: &mut Vec<BoundDiscoverySocket>,
    receive_tasks: &mut Vec<JoinHandle<()>>,
    datagram_tx: &mpsc::Sender<ReceivedDatagram>,
) -> Result<bool> {
    let current = current_selected_endpoints(preferences)?;
    if current != *endpoints {
        replace_sockets(multicast_v4, &current, sockets, receive_tasks, datagram_tx);
        *endpoints = current;
        return Ok(true);
    }
    Ok(false)
}

fn current_selected_endpoints(
    preferences: &Arc<RwLock<NetworkPreferences>>,
) -> io::Result<Vec<DiscoveryEndpoint>> {
    let preferences = preferences
        .read()
        .expect("network preferences lock should not be poisoned")
        .clone();
    selected_endpoints(&preferences)
}

fn replace_sockets(
    multicast_v4: Ipv4Addr,
    endpoints: &[DiscoveryEndpoint],
    sockets: &mut Vec<BoundDiscoverySocket>,
    receive_tasks: &mut Vec<JoinHandle<()>>,
    datagram_tx: &mpsc::Sender<ReceivedDatagram>,
) {
    for task in receive_tasks.drain(..) {
        task.abort();
    }
    *sockets = bind_discovery_sockets(multicast_v4, endpoints);
    *receive_tasks = spawn_receive_tasks(sockets, datagram_tx.clone());
    log_active_interfaces(sockets);
}

fn bind_discovery_sockets(
    multicast_v4: Ipv4Addr,
    endpoints: &[DiscoveryEndpoint],
) -> Vec<BoundDiscoverySocket> {
    endpoints
        .iter()
        .filter_map(|endpoint| match bind_discovery_socket(multicast_v4, endpoint) {
            Ok(socket) => Some(socket),
            Err(error) => {
                warn!(interface = %endpoint.description(), %error, "failed to bind LocalSend multicast socket");
                None
            }
        })
        .collect()
}

fn bind_discovery_socket(
    multicast_v4: Ipv4Addr,
    endpoint: &DiscoveryEndpoint,
) -> io::Result<BoundDiscoverySocket> {
    match endpoint {
        DiscoveryEndpoint::V4 { address, .. } => {
            let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(SocketProtocol::UDP))?;
            socket.set_reuse_address(true)?;
            #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
            socket.set_reuse_port(true)?;
            socket.bind(
                &SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::UNSPECIFIED,
                    DEFAULT_MULTICAST_PORT,
                ))
                .into(),
            )?;
            socket.join_multicast_v4(&multicast_v4, address)?;
            socket.set_multicast_if_v4(address)?;
            socket.set_multicast_loop_v4(true)?;
            socket.set_multicast_ttl_v4(1)?;
            socket.set_nonblocking(true)?;
            Ok(BoundDiscoverySocket {
                endpoint: endpoint.clone(),
                target: SocketAddr::V4(SocketAddrV4::new(multicast_v4, DEFAULT_MULTICAST_PORT)),
                socket: Arc::new(UdpSocket::from_std(socket.into())?),
            })
        }
        DiscoveryEndpoint::V6 { index, .. } => {
            let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(SocketProtocol::UDP))?;
            socket.set_only_v6(true)?;
            socket.set_reuse_address(true)?;
            #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
            socket.set_reuse_port(true)?;
            socket.bind(
                &SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::UNSPECIFIED,
                    DEFAULT_MULTICAST_PORT,
                    0,
                    0,
                ))
                .into(),
            )?;
            socket.join_multicast_v6(&DEFAULT_MULTICAST_GROUP_V6, *index)?;
            socket.set_multicast_if_v6(*index)?;
            socket.set_multicast_loop_v6(true)?;
            socket.set_multicast_hops_v6(1)?;
            socket.set_nonblocking(true)?;
            Ok(BoundDiscoverySocket {
                endpoint: endpoint.clone(),
                target: SocketAddr::V6(SocketAddrV6::new(
                    DEFAULT_MULTICAST_GROUP_V6,
                    DEFAULT_MULTICAST_PORT,
                    0,
                    *index,
                )),
                socket: Arc::new(UdpSocket::from_std(socket.into())?),
            })
        }
    }
}

fn spawn_receive_tasks(
    sockets: &[BoundDiscoverySocket],
    datagram_tx: mpsc::Sender<ReceivedDatagram>,
) -> Vec<JoinHandle<()>> {
    sockets
        .iter()
        .map(|bound| {
            let endpoint = bound.endpoint.clone();
            let socket = bound.socket.clone();
            let datagram_tx = datagram_tx.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0_u8; 65_536];
                loop {
                    match socket.recv_from(&mut buffer).await {
                        Ok((length, source)) => {
                            if datagram_tx
                                .send(ReceivedDatagram {
                                    endpoint: endpoint.clone(),
                                    source,
                                    payload: buffer[..length].to_vec(),
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            warn!(interface = %endpoint.description(), %error, "LocalSend multicast receive failed");
                            break;
                        }
                    }
                }
            })
        })
        .collect()
}

fn handle_announcement(
    payload: &[u8],
    source: SocketAddr,
    source_interface: &str,
    local_device: &DeviceInfo,
    devices: &Arc<RwLock<HashMap<String, SeenDevice>>>,
) -> Option<(bool, String)> {
    let announcement = serde_json::from_slice::<AnnouncementMessage>(payload).ok()?;
    if announcement.fingerprint == local_device.fingerprint {
        return None;
    }
    let host = match source {
        SocketAddr::V6(address) if address.scope_id() != 0 => {
            format!("{}%{}", address.ip(), address.scope_id())
        }
        _ => source.ip().to_string(),
    };
    let source_interface = match source {
        SocketAddr::V6(address) if address.scope_id() != 0 => {
            interface_name_by_index(address.scope_id())
        }
        _ => route_interface_for_ip(source.ip()).unwrap_or(None),
    }
    .unwrap_or_else(|| source_interface.to_owned());
    {
        let known = devices
            .read()
            .expect("device discovery lock should not be poisoned");
        if known.get(&announcement.fingerprint).is_some_and(|seen| {
            seen.device.ip.as_deref() == Some(host.as_str())
                && seen.last_seen.elapsed() < Duration::from_secs(1)
        }) {
            return None;
        }
    }
    let device = DeviceInfo {
        alias: announcement.alias,
        version: announcement.version,
        device_model: announcement.device_model,
        device_type: announcement.device_type,
        fingerprint: announcement.fingerprint,
        port: announcement.port,
        protocol: announcement.protocol,
        download: announcement.download,
        ip: Some(host),
    };
    debug!(alias = %device.alias, address = %source.ip(), interface = source_interface, "discovered LocalSend device");
    devices
        .write()
        .expect("device discovery lock should not be poisoned")
        .insert(
            device.fingerprint.clone(),
            SeenDevice {
                device,
                last_seen: Instant::now(),
                source_interface: Some(source_interface.clone()),
            },
        );
    Some((
        announcement.announce || announcement.announcement.unwrap_or(false),
        source_interface,
    ))
}

fn interface_name_by_index(index: u32) -> Option<String> {
    get_if_addrs()
        .ok()?
        .into_iter()
        .find(|interface| interface.index == Some(index))
        .map(|interface| interface.name)
}

async fn send_announcement(
    local_device: &DeviceInfo,
    sockets: &[BoundDiscoverySocket],
    announce: bool,
) {
    let message = AnnouncementMessage {
        alias: local_device.alias.clone(),
        version: local_device.version.clone(),
        device_model: local_device.device_model.clone(),
        device_type: local_device.device_type,
        fingerprint: local_device.fingerprint.clone(),
        port: local_device.port,
        protocol: local_device.protocol,
        download: local_device.download,
        announce,
        announcement: Some(announce),
    };
    let Ok(payload) = serde_json::to_vec(&message) else {
        warn!("failed to encode LocalSend announcement");
        return;
    };
    if sockets.is_empty() {
        warn!("no multicast-capable network interface is active");
        return;
    }

    for delay in [
        Duration::from_millis(100),
        Duration::from_millis(500),
        Duration::from_millis(2_000),
    ] {
        tokio::time::sleep(delay).await;
        for bound in sockets {
            if let Err(error) = bound.socket.send_to(&payload, bound.target).await {
                warn!(interface = %bound.endpoint.description(), %error, "failed to send LocalSend announcement");
            }
        }
    }
}

fn log_active_interfaces(sockets: &[BoundDiscoverySocket]) {
    let interfaces = sockets
        .iter()
        .map(|socket| socket.endpoint.description())
        .collect::<Vec<_>>();
    info!(?interfaces, "LocalSend multicast interfaces updated");
}

pub fn route_interface_for_ip(target: IpAddr) -> io::Result<Option<String>> {
    let bind_address = match target {
        IpAddr::V4(_) => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
        IpAddr::V6(_) => SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)),
    };
    let target_address = SocketAddr::new(target, DEFAULT_MULTICAST_PORT);
    let socket = std::net::UdpSocket::bind(bind_address)?;
    socket.connect(target_address)?;
    let local_ip = socket.local_addr()?.ip();
    Ok(get_if_addrs()?
        .into_iter()
        .find(|interface| interface.ip() == local_ip)
        .map(|interface| interface.name))
}

pub async fn run_ipv6_tcp_proxy(port: u16) -> Result<()> {
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(SocketProtocol::TCP))?;
    socket.set_only_v6(true)?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0)).into())?;
    socket.listen(128)?;
    socket.set_nonblocking(true)?;
    let listener = TcpListener::from_std(socket.into())?;
    info!(port, "LocalSend IPv6 proxy is ready");

    loop {
        let (mut inbound, source) = listener.accept().await?;
        tokio::spawn(async move {
            match TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
                Ok(mut upstream) => {
                    if let Err(error) = copy_bidirectional(&mut inbound, &mut upstream).await {
                        debug!(%source, %error, "IPv6 LocalSend proxy connection ended");
                    }
                }
                Err(error) => warn!(%source, %error, "failed to connect IPv6 proxy upstream"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{InterfaceKind, NetworkMode, NetworkSelection, interface_kind};

    #[test]
    fn classifies_common_host_interfaces() {
        assert_eq!(interface_kind("enp2s0"), InterfaceKind::Ethernet);
        assert_eq!(interface_kind("wlp129s0"), InterfaceKind::Wifi);
        assert_eq!(interface_kind("br-aabbcc"), InterfaceKind::Bridge);
        assert_eq!(interface_kind("heiyu-0"), InterfaceKind::Tunnel);
    }

    #[test]
    fn selection_defaults_to_every_capable_interface() {
        let all = NetworkSelection::all();
        assert_eq!(all.mode, NetworkMode::All);
        assert!(all.includes("enp2s0"));

        let selected = NetworkSelection::selected(BTreeSet::from(["wlp129s0".to_owned()]));
        assert!(!selected.includes("enp2s0"));
        assert!(selected.includes("wlp129s0"));
    }
}
