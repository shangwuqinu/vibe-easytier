use std::{
    fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;
use vibe_easytier_service::service::{CORE_CONFIG_VALIDATION_ERROR_PREFIX, WINDOWS_SERVICE_NAME};
use vibe_easytier_service::{
    bandwidth::{run_iperf3_test, DEFAULT_TEST_DURATION_SECONDS},
    AddressMode, ConnectedPeer, ConnectionIntent, EasyTierFlags, IpcClient, NetworkProfile,
    ProfileUpsert, RpcCommand, RpcRequest, RpcResult, SecretString, ServiceConnectionState,
    ServiceStatus,
};

const MAX_LOG_LINES: usize = 200;
const MAX_IMPORT_TOML_BYTES: usize = 1024 * 1024;
static NEXT_LOG_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopPreferences {
    #[serde(default = "default_theme")]
    theme: String,
}

fn default_theme() -> String {
    "system".to_owned()
}

struct NativeState {
    client: IpcClient,
    next_request_id: AtomicU64,
    preferences_path: PathBuf,
    iperf3_executable: PathBuf,
    preferences: Mutex<DesktopPreferences>,
    desktop_logs: Mutex<Vec<UiLog>>,
}

impl NativeState {
    fn load(preferences_path: PathBuf, iperf3_executable: PathBuf) -> Self {
        let preferences = fs::read(&preferences_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            client: IpcClient::windows_default(),
            next_request_id: AtomicU64::new(1),
            preferences_path,
            iperf3_executable,
            preferences: Mutex::new(preferences),
            desktop_logs: Mutex::new(Vec::new()),
        }
    }

    fn rpc(&self, command: RpcCommand) -> Result<RpcResult, String> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .client
            .call(&RpcRequest::new(request_id, command))
            .map_err(|_| "无法连接后台服务，请确认 VibeEasyTierService 正在运行。".to_owned())?;
        if let Some(error) = response.error {
            return Err(localize_service_error(&error.message));
        }
        response
            .result
            .ok_or_else(|| "后台服务返回了空响应。".to_owned())
    }

    fn service_status(&self) -> Result<ServiceStatus, String> {
        match self.rpc(RpcCommand::GetStatus)? {
            RpcResult::Status(status) => Ok(status),
            _ => Err("后台服务返回了无效的状态响应。".to_owned()),
        }
    }

    fn profile_views(&self) -> Result<Vec<vibe_easytier_service::ProfileView>, String> {
        match self.rpc(RpcCommand::ListProfiles)? {
            RpcResult::Profiles(profiles) => Ok(profiles),
            _ => Err("后台服务返回了无效的档案响应。".to_owned()),
        }
    }

    fn connected_peers(&self) -> Result<Vec<ConnectedPeer>, String> {
        match self.rpc(RpcCommand::ListPeers)? {
            RpcResult::Peers(peers) => Ok(peers),
            _ => Err("后台服务返回了无效的节点响应。".to_owned()),
        }
    }

    fn desktop_logs(&self) -> Vec<UiLog> {
        self.desktop_logs
            .lock()
            .map(|logs| logs.clone())
            .unwrap_or_default()
    }

    fn theme(&self) -> String {
        self.preferences
            .lock()
            .map(|preferences| preferences.theme.clone())
            .unwrap_or_else(|_| default_theme())
    }

    fn set_theme(&self, theme: String) -> Result<(), String> {
        if !matches!(theme.as_str(), "system" | "light" | "dark") {
            return Err("主题仅支持系统、浅色或深色。".to_owned());
        }
        let serialized = {
            let mut preferences = self
                .preferences
                .lock()
                .map_err(|_| "桌面偏好设置暂时不可用。".to_owned())?;
            preferences.theme = theme;
            serde_json::to_vec_pretty(&*preferences)
                .map_err(|_| "无法保存桌面偏好设置。".to_owned())?
        };
        atomic_write(&self.preferences_path, &serialized)
            .map_err(|_| "无法保存桌面偏好设置。".to_owned())
    }
}

/// Browser-facing representation of every EasyTier Core 2.6.4 `[flags]`
/// field.  The service keeps its native snake_case schema; this boundary is
/// deliberately camelCase for the React application.
///
/// JavaScript cannot represent every `u64` exactly, so the two BPS limits are
/// sent as decimal strings. An empty input is accepted as the Core's
/// unlimited sentinel when converting back to the service type.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiEasyTierFlags {
    default_protocol: String,
    dev_name: String,
    enable_encryption: bool,
    enable_ipv6: bool,
    mtu: u32,
    latency_first: bool,
    enable_exit_node: bool,
    no_tun: bool,
    use_smoltcp: bool,
    relay_network_whitelist: String,
    disable_p2p: bool,
    relay_all_peer_rpc: bool,
    disable_udp_hole_punching: bool,
    multi_thread: bool,
    data_compress_algo: String,
    bind_device: bool,
    enable_kcp_proxy: bool,
    disable_kcp_input: bool,
    disable_relay_kcp: bool,
    proxy_forward_by_system: bool,
    accept_dns: bool,
    private_mode: bool,
    enable_quic_proxy: bool,
    disable_quic_input: bool,
    disable_relay_quic: bool,
    quic_listen_port: u32,
    foreign_relay_bps_limit: String,
    multi_thread_count: u32,
    enable_relay_foreign_network_kcp: bool,
    enable_relay_foreign_network_quic: bool,
    encryption_algorithm: String,
    disable_sym_hole_punching: bool,
    tld_dns_zone: String,
    p2p_only: bool,
    disable_tcp_hole_punching: bool,
    lazy_p2p: bool,
    need_p2p: bool,
    instance_recv_bps_limit: String,
    disable_upnp: bool,
    disable_relay_data: bool,
    enable_udp_broadcast_relay: bool,
}

impl Default for UiEasyTierFlags {
    fn default() -> Self {
        Self::from(EasyTierFlags::default())
    }
}

impl From<EasyTierFlags> for UiEasyTierFlags {
    fn from(flags: EasyTierFlags) -> Self {
        Self {
            default_protocol: flags.default_protocol,
            dev_name: flags.dev_name,
            enable_encryption: flags.enable_encryption,
            enable_ipv6: flags.enable_ipv6,
            mtu: flags.mtu,
            latency_first: flags.latency_first,
            enable_exit_node: flags.enable_exit_node,
            no_tun: flags.no_tun,
            use_smoltcp: flags.use_smoltcp,
            relay_network_whitelist: flags.relay_network_whitelist,
            disable_p2p: flags.disable_p2p,
            relay_all_peer_rpc: flags.relay_all_peer_rpc,
            disable_udp_hole_punching: flags.disable_udp_hole_punching,
            multi_thread: flags.multi_thread,
            // Stored profiles are validated by the service. Preserve a safe
            // UI value if an old/corrupted state predates that invariant.
            data_compress_algo: match flags.data_compress_algo {
                2 => "zstd".to_owned(),
                _ => "none".to_owned(),
            },
            bind_device: flags.bind_device,
            enable_kcp_proxy: flags.enable_kcp_proxy,
            disable_kcp_input: flags.disable_kcp_input,
            disable_relay_kcp: flags.disable_relay_kcp,
            proxy_forward_by_system: flags.proxy_forward_by_system,
            accept_dns: flags.accept_dns,
            private_mode: flags.private_mode,
            enable_quic_proxy: flags.enable_quic_proxy,
            disable_quic_input: flags.disable_quic_input,
            disable_relay_quic: flags.disable_relay_quic,
            quic_listen_port: flags.quic_listen_port,
            foreign_relay_bps_limit: flags.foreign_relay_bps_limit.to_string(),
            multi_thread_count: flags.multi_thread_count,
            enable_relay_foreign_network_kcp: flags.enable_relay_foreign_network_kcp,
            enable_relay_foreign_network_quic: flags.enable_relay_foreign_network_quic,
            encryption_algorithm: flags.encryption_algorithm,
            disable_sym_hole_punching: flags.disable_sym_hole_punching,
            tld_dns_zone: flags.tld_dns_zone,
            p2p_only: flags.p2p_only,
            disable_tcp_hole_punching: flags.disable_tcp_hole_punching,
            lazy_p2p: flags.lazy_p2p,
            need_p2p: flags.need_p2p,
            instance_recv_bps_limit: flags.instance_recv_bps_limit.to_string(),
            disable_upnp: flags.disable_upnp,
            disable_relay_data: flags.disable_relay_data,
            enable_udp_broadcast_relay: flags.enable_udp_broadcast_relay,
        }
    }
}

impl TryFrom<UiEasyTierFlags> for EasyTierFlags {
    type Error = String;

    fn try_from(flags: UiEasyTierFlags) -> Result<Self, Self::Error> {
        let data_compress_algo = match flags.data_compress_algo.trim() {
            "none" => 1,
            "zstd" => 2,
            _ => return Err("数据压缩仅支持“不压缩”或 Zstandard。".to_owned()),
        };

        Ok(Self {
            default_protocol: flags.default_protocol,
            dev_name: flags.dev_name,
            enable_encryption: flags.enable_encryption,
            enable_ipv6: flags.enable_ipv6,
            mtu: flags.mtu,
            latency_first: flags.latency_first,
            enable_exit_node: flags.enable_exit_node,
            no_tun: flags.no_tun,
            use_smoltcp: flags.use_smoltcp,
            relay_network_whitelist: flags.relay_network_whitelist,
            disable_p2p: flags.disable_p2p,
            relay_all_peer_rpc: flags.relay_all_peer_rpc,
            disable_udp_hole_punching: flags.disable_udp_hole_punching,
            multi_thread: flags.multi_thread,
            data_compress_algo,
            bind_device: flags.bind_device,
            enable_kcp_proxy: flags.enable_kcp_proxy,
            disable_kcp_input: flags.disable_kcp_input,
            disable_relay_kcp: flags.disable_relay_kcp,
            proxy_forward_by_system: flags.proxy_forward_by_system,
            accept_dns: flags.accept_dns,
            private_mode: flags.private_mode,
            enable_quic_proxy: flags.enable_quic_proxy,
            disable_quic_input: flags.disable_quic_input,
            disable_relay_quic: flags.disable_relay_quic,
            quic_listen_port: flags.quic_listen_port,
            foreign_relay_bps_limit: parse_bps_limit(
                &flags.foreign_relay_bps_limit,
                "外部网络中继速率上限",
            )?,
            multi_thread_count: flags.multi_thread_count,
            enable_relay_foreign_network_kcp: flags.enable_relay_foreign_network_kcp,
            enable_relay_foreign_network_quic: flags.enable_relay_foreign_network_quic,
            encryption_algorithm: flags.encryption_algorithm,
            disable_sym_hole_punching: flags.disable_sym_hole_punching,
            tld_dns_zone: flags.tld_dns_zone,
            p2p_only: flags.p2p_only,
            disable_tcp_hole_punching: flags.disable_tcp_hole_punching,
            lazy_p2p: flags.lazy_p2p,
            need_p2p: flags.need_p2p,
            instance_recv_bps_limit: parse_bps_limit(
                &flags.instance_recv_bps_limit,
                "实例接收速率上限",
            )?,
            disable_upnp: flags.disable_upnp,
            disable_relay_data: flags.disable_relay_data,
            enable_udp_broadcast_relay: flags.enable_udp_broadcast_relay,
        })
    }
}

fn parse_bps_limit(value: &str, label: &str) -> Result<u64, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(u64::MAX);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{label}必须是十进制非负整数，留空表示不限制。"));
    }
    let value = value
        .parse::<u64>()
        .map_err(|_| format!("{label}超出了 64 位无符号整数范围。"))?;
    if value != u64::MAX && value > i64::MAX as u64 {
        return Err(format!("{label}不能超过 {}，留空表示不限制。", i64::MAX));
    }
    Ok(value)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiNetworkProfile {
    id: String,
    name: String,
    device_name: String,
    network_name: String,
    network_secret: String,
    peers: Vec<String>,
    virtual_ip: String,
    #[serde(default)]
    flags: UiEasyTierFlags,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiProfile {
    id: String,
    name: String,
    device_name: String,
    network_name: String,
    network_secret: String,
    peers: Vec<String>,
    virtual_ip: String,
    flags: UiEasyTierFlags,
    updated_at: String,
}

impl From<vibe_easytier_service::ProfileView> for UiProfile {
    fn from(profile: vibe_easytier_service::ProfileView) -> Self {
        let virtual_ip = profile.static_ipv4_cidr.unwrap_or_default();
        Self {
            id: profile.id,
            name: profile.name,
            device_name: profile.hostname,
            network_name: profile.network_name,
            // A secret must be supplied again for an edit; the service never
            // returns it to a desktop process or serializes it into UI state.
            network_secret: String::new(),
            peers: profile.peers,
            virtual_ip,
            flags: profile.flags.into(),
            updated_at: now_iso(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopSnapshot {
    profiles: Vec<UiProfile>,
    peers: Vec<UiPeer>,
    runtime: UiRuntime,
    preferences: UiPreferences,
    logs: Vec<UiLog>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiRuntime {
    phase: String,
    active_profile_id: Option<String>,
    started_at: Option<String>,
    retry_at: Option<String>,
    error: Option<String>,
    peer_count: usize,
    peer_count_available: bool,
    last_success_at: Option<String>,
    routes: u32,
    sent: u64,
    received: u64,
    daemon_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiPreferences {
    auto_connect: bool,
    service_at_boot: bool,
    service_health: String,
    theme: String,
}

#[derive(Clone, Copy, Debug)]
struct ServiceBootStatus {
    configured: bool,
    running: bool,
}

impl ServiceBootStatus {
    fn health(self) -> &'static str {
        if self.configured && self.running {
            "healthy"
        } else if self.configured {
            "attention"
        } else {
            "unavailable"
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiPeer {
    id: String,
    name: String,
    hostname: String,
    virtual_ip: String,
    /// Active direct transport protocols reported by the service. This is not
    /// the configured Bootstrap URI list: a peer can have more than one live
    /// connection at the same time.
    protocols: Vec<String>,
    role: String,
    state: String,
    latency_ms: u32,
    last_seen: String,
    version: String,
    sent: u64,
    received: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiBandwidthTestResult {
    peer_id: String,
    download_bps: u64,
    upload_bps: u64,
    download_bytes: u64,
    upload_bytes: u64,
    duration_seconds: u8,
    tested_at: String,
}

impl From<ConnectedPeer> for UiPeer {
    fn from(peer: ConnectedPeer) -> Self {
        let name = peer.hostname.clone();
        Self {
            id: peer.id,
            name,
            hostname: peer.hostname,
            virtual_ip: peer.ipv4,
            protocols: peer.protocols,
            role: "Peer".to_owned(),
            state: "online".to_owned(),
            latency_ms: peer.latency_ms.unwrap_or_default(),
            last_seen: now_iso(),
            version: peer.version.unwrap_or_else(|| "Unknown".to_owned()),
            sent: peer.tx_bytes.unwrap_or_default(),
            received: peer.rx_bytes.unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiLog {
    id: String,
    at: String,
    level: String,
    source: String,
    message: String,
}

impl UiLog {
    fn new(
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: format!(
                "{}-{}-{}",
                unix_millis(),
                std::process::id(),
                NEXT_LOG_ID.fetch_add(1, Ordering::Relaxed)
            ),
            at: now_iso(),
            level: level.into(),
            source: source.into(),
            message: message.into(),
        }
    }
}

#[tauri::command]
fn get_snapshot(state: State<'_, NativeState>) -> DesktopSnapshot {
    let theme = state.theme();
    let mut service_boot = service_boot_status();
    let status = match state.service_status() {
        Ok(status) => {
            // A successful authenticated service RPC is stronger evidence of
            // liveness than parsing localized `sc.exe` output.
            service_boot.running = true;
            status
        }
        Err(error) => {
            return unavailable_snapshot(theme, service_boot, error, state.desktop_logs());
        }
    };
    let profiles = match state.profile_views() {
        Ok(profiles) => profiles.into_iter().map(UiProfile::from).collect(),
        Err(error) => {
            return unavailable_snapshot(theme, service_boot, error, state.desktop_logs());
        }
    };
    let peers = state
        .connected_peers()
        .unwrap_or_default()
        .into_iter()
        .map(UiPeer::from)
        .collect();
    let mut logs = service_logs(&state).unwrap_or_default();
    logs.extend(state.desktop_logs());
    logs.truncate(MAX_LOG_LINES);

    DesktopSnapshot {
        profiles,
        peers,
        runtime: UiRuntime {
            phase: ui_phase(status.state).to_owned(),
            active_profile_id: status.active_profile_id,
            started_at: None,
            retry_at: status.retry_at_unix_ms.and_then(timestamp_to_iso),
            error: status.last_error.as_deref().map(localize_service_error),
            peer_count: status.peer_count,
            peer_count_available: status.peer_count_available,
            last_success_at: status.last_success_unix_ms.and_then(timestamp_to_iso),
            routes: status.route_count.min(u32::MAX as usize) as u32,
            sent: status.traffic_tx_bytes,
            received: status.traffic_rx_bytes,
            daemon_version: "EasyTier Core 2.6.4（由服务管理）".to_owned(),
        },
        preferences: UiPreferences {
            auto_connect: status.auto_connect_profile_id.is_some(),
            service_at_boot: service_boot.configured,
            service_health: service_boot.health().to_owned(),
            theme,
        },
        logs,
    }
}

#[tauri::command]
fn save_profile(
    state: State<'_, NativeState>,
    profile: UiNetworkProfile,
) -> Result<UiProfile, String> {
    if profile.network_secret.trim().is_empty() {
        return Err("保存档案需要填写网络密钥。".to_owned());
    }
    let flags = EasyTierFlags::try_from(profile.flags)?;
    let active_profile_id = state.service_status()?.active_profile_id;
    let existing_auto_connect = state
        .profile_views()?
        .into_iter()
        .find(|existing| existing.id == profile.id)
        .map(|existing| existing.auto_connect)
        .unwrap_or(false);
    let service_profile = NetworkProfile {
        id: profile.id.clone(),
        name: profile.name,
        instance_name: managed_instance_name(&profile.id),
        hostname: profile.device_name,
        network_name: profile.network_name,
        network_secret: SecretString::new(profile.network_secret.clone()),
        address_mode: AddressMode::Static {
            cidr: profile.virtual_ip,
        },
        peers: profile.peers,
        flags,
        auto_connect: existing_auto_connect,
    };
    let saved = state.rpc(RpcCommand::UpsertProfile(ProfileUpsert {
        make_active: active_profile_id.as_deref() == Some(service_profile.id.as_str()),
        profile: service_profile,
    }))?;
    match saved {
        RpcResult::ProfileSaved(saved) => Ok(saved.into()),
        _ => Err("后台服务返回了无效的档案保存响应。".to_owned()),
    }
}

/// Updates an existing profile's typed Core flags. The service keeps the
/// network secret in its DPAPI-protected state; this command never requests,
/// receives, or sends it back to the webview.
#[tauri::command]
fn update_profile_flags(
    state: State<'_, NativeState>,
    profile_id: String,
    flags: UiEasyTierFlags,
) -> Result<UiProfile, String> {
    let flags = EasyTierFlags::try_from(flags)?;
    match state.rpc(RpcCommand::UpdateProfileFlags { profile_id, flags })? {
        RpcResult::ProfileSaved(profile) => Ok(profile.into()),
        _ => Err("后台服务返回了无效的 Core 选项保存响应。".to_owned()),
    }
}

fn import_profile_toml(state: &NativeState, toml: String) -> Result<UiProfile, String> {
    let make_active = state.service_status()?.active_profile_id.is_none();
    match state.rpc(RpcCommand::ImportProfile { toml, make_active })? {
        RpcResult::ProfileSaved(profile) => Ok(profile.into()),
        _ => Err("后台服务返回了无效的档案导入响应。".to_owned()),
    }
}

#[tauri::command]
async fn import_profile_from_file(
    app: tauri::AppHandle,
    state: State<'_, NativeState>,
) -> Result<Option<UiProfile>, String> {
    let selected = app
        .dialog()
        .file()
        .add_filter("EasyTier TOML", &["toml"])
        .set_title("从本地导入 EasyTier TOML")
        .blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|_| "选择的 TOML 文件路径无效。".to_owned())?;
    let toml = read_import_toml(&path)?;
    import_profile_toml(&state, toml).map(Some)
}

fn read_import_toml(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|_| "无法读取所选 TOML 文件，请确认文件仍然存在且具有读取权限。".to_owned())?;
    if metadata.len() > MAX_IMPORT_TOML_BYTES as u64 {
        return Err("所选 TOML 文件超过 1 MB，无法导入。".to_owned());
    }
    let bytes = fs::read(path)
        .map_err(|_| "无法读取所选 TOML 文件，请确认文件仍然存在且具有读取权限。".to_owned())?;
    decode_import_toml(bytes)
}

fn decode_import_toml(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > MAX_IMPORT_TOML_BYTES {
        return Err("所选 TOML 文件超过 1 MB，无法导入。".to_owned());
    }
    let toml =
        String::from_utf8(bytes).map_err(|_| "所选 TOML 文件不是有效的 UTF-8 文本。".to_owned())?;
    Ok(toml.strip_prefix('\u{feff}').unwrap_or(&toml).to_owned())
}

#[tauri::command]
async fn export_profile_toml(
    app: tauri::AppHandle,
    state: State<'_, NativeState>,
    profile_id: String,
) -> Result<Option<String>, String> {
    let profile = state
        .profile_views()?
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "要导出的档案不存在。".to_owned())?;
    let selected = app
        .dialog()
        .file()
        .add_filter("EasyTier TOML", &["toml"])
        .set_title("导出完整配置（包含网络密钥）")
        .set_file_name(export_file_name(&profile.name))
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let mut path = selected
        .into_path()
        .map_err(|_| "选择的导出路径无效。".to_owned())?;
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
    {
        path.set_extension("toml");
    }

    let toml = match state.rpc(RpcCommand::ExportProfile {
        profile_id: profile.id.clone(),
    })? {
        RpcResult::ProfileToml {
            profile_id: exported_id,
            toml,
        } if exported_id == profile.id => toml,
        _ => return Err("后台服务返回了无效的档案导出响应。".to_owned()),
    };
    fs::write(&path, toml.as_bytes()).map_err(|_| "无法写入导出的 TOML 文件。".to_owned())?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
async fn run_peer_bandwidth_test(
    state: State<'_, NativeState>,
    peer_id: String,
) -> Result<UiBandwidthTestResult, String> {
    let status = state.service_status()?;
    if status.state != ServiceConnectionState::Connected {
        return Err("私有网络尚未连接，无法进行节点间带宽测试。".to_owned());
    }
    let profiles = state.profile_views()?;
    let local_address = active_virtual_ipv4(&status, &profiles)?;
    let peer = state
        .connected_peers()?
        .into_iter()
        .find(|peer| peer.id == peer_id)
        .ok_or_else(|| "目标节点已离线或不存在。".to_owned())?;
    let peer_address = peer
        .ipv4
        .split('/')
        .next()
        .and_then(|address| address.parse().ok())
        .ok_or_else(|| "目标节点没有可用的虚拟 IPv4 地址。".to_owned())?;
    if peer_address == local_address {
        return Err("不能对本机节点执行带宽测试。".to_owned());
    }
    let iperf3_executable = state.iperf3_executable.clone();

    let measurement = tokio::task::spawn_blocking(move || {
        run_iperf3_test(
            &iperf3_executable,
            local_address,
            peer_address,
            DEFAULT_TEST_DURATION_SECONDS,
        )
    })
    .await
    .map_err(|_| "带宽测试任务异常结束。".to_owned())?
    .map_err(localize_bandwidth_error)?;

    Ok(UiBandwidthTestResult {
        peer_id,
        download_bps: measurement.download_bps,
        upload_bps: measurement.upload_bps,
        download_bytes: measurement.download_bytes,
        upload_bytes: measurement.upload_bytes,
        duration_seconds: measurement.duration_seconds,
        tested_at: now_iso(),
    })
}

fn active_virtual_ipv4(
    status: &ServiceStatus,
    profiles: &[vibe_easytier_service::ProfileView],
) -> Result<Ipv4Addr, String> {
    let active_profile_id = status
        .active_profile_id
        .as_deref()
        .ok_or_else(|| "当前没有活动私有网络档案。".to_owned())?;
    let cidr = profiles
        .iter()
        .find(|profile| profile.id == active_profile_id)
        .and_then(|profile| profile.static_ipv4_cidr.as_deref())
        .ok_or_else(|| "活动档案没有可用的固定虚拟 IPv4 地址。".to_owned())?;
    cidr.split('/')
        .next()
        .and_then(|address| address.parse().ok())
        .ok_or_else(|| "活动档案的虚拟 IPv4 地址无效。".to_owned())
}

#[tauri::command]
fn delete_profile(state: State<'_, NativeState>, profile_id: String) -> Result<(), String> {
    match state.rpc(RpcCommand::DeleteProfile { profile_id })? {
        RpcResult::ProfileDeleted { .. } => Ok(()),
        _ => Err("后台服务返回了无效的档案删除响应。".to_owned()),
    }
}

#[tauri::command]
fn select_active_profile(state: State<'_, NativeState>, profile_id: String) -> Result<(), String> {
    match state.rpc(RpcCommand::SetActiveProfile { profile_id })? {
        RpcResult::ActiveProfileSelected(_) => Ok(()),
        _ => Err("后台服务返回了无效的活动档案选择响应。".to_owned()),
    }
}

#[tauri::command]
fn connect(state: State<'_, NativeState>, profile_id: String) -> Result<(), String> {
    apply_connection_intent(&state, ConnectionIntent::Connect { profile_id })
}

#[tauri::command]
fn disconnect(state: State<'_, NativeState>) -> Result<(), String> {
    // Disconnect is a durable intent: it clears automatic reconnection before
    // the child is stopped, so the service never raises it behind the user.
    apply_connection_intent(&state, ConnectionIntent::Disconnect { profile_id: None })
}

#[tauri::command]
fn set_auto_connect(state: State<'_, NativeState>, enabled: bool) -> Result<(), String> {
    let profile_id = state
        .service_status()?
        .active_profile_id
        .ok_or_else(|| "请先选择一个私有网络，再设置自动连接。".to_owned())?;
    apply_connection_intent(
        &state,
        ConnectionIntent::SetAutoConnect {
            profile_id,
            enabled,
        },
    )
}

#[tauri::command]
fn set_theme(state: State<'_, NativeState>, theme: String) -> Result<(), String> {
    state.set_theme(theme)
}

#[tauri::command]
fn clear_logs(state: State<'_, NativeState>) -> Result<(), String> {
    match state.rpc(RpcCommand::ClearLogs)? {
        RpcResult::LogsCleared => {}
        _ => return Err("后台服务返回了无效的日志清理响应。".to_owned()),
    }
    let mut logs = state
        .desktop_logs
        .lock()
        .map_err(|_| "桌面日志暂时不可用。".to_owned())?;
    logs.clear();
    Ok(())
}

#[tauri::command]
fn minimize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|_| "无法最小化窗口。".to_owned())
}

#[tauri::command]
fn toggle_maximize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    if window
        .is_maximized()
        .map_err(|_| "无法读取窗口状态。".to_owned())?
    {
        window.unmaximize().map_err(|_| "无法还原窗口。".to_owned())
    } else {
        window.maximize().map_err(|_| "无法最大化窗口。".to_owned())
    }
}

#[tauri::command]
fn hide_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|_| "无法隐藏窗口。".to_owned())
}

fn apply_connection_intent(state: &NativeState, intent: ConnectionIntent) -> Result<(), String> {
    match state.rpc(RpcCommand::SetConnectionIntent { intent })? {
        RpcResult::IntentApplied(_) => Ok(()),
        _ => Err("后台服务返回了无效的连接意图响应。".to_owned()),
    }
}

fn service_logs(state: &NativeState) -> Result<Vec<UiLog>, String> {
    match state.rpc(RpcCommand::TailLogs {
        limit: MAX_LOG_LINES,
    })? {
        RpcResult::Logs(lines) => Ok(lines
            .into_iter()
            .map(|line| UiLog::new(core_log_level(&line.line), "EasyTier Core", line.line))
            .collect()),
        _ => Err("后台服务返回了无效的日志响应。".to_owned()),
    }
}

fn core_log_level(message: &str) -> &'static str {
    for token in message
        .lines()
        .next()
        .unwrap_or_default()
        .split(|character: char| !character.is_ascii_alphabetic())
        .filter(|token| !token.is_empty())
    {
        if token.eq_ignore_ascii_case("error") {
            return "error";
        }
        if token.eq_ignore_ascii_case("warn") || token.eq_ignore_ascii_case("warning") {
            return "warning";
        }
        if token.eq_ignore_ascii_case("info")
            || token.eq_ignore_ascii_case("debug")
            || token.eq_ignore_ascii_case("trace")
        {
            return "info";
        }
    }
    "info"
}

fn unavailable_snapshot(
    theme: String,
    service_boot: ServiceBootStatus,
    error: String,
    mut logs: Vec<UiLog>,
) -> DesktopSnapshot {
    logs.insert(
        0,
        UiLog::new("warning", "Desktop", format!("后台服务状态不可用：{error}")),
    );
    logs.truncate(MAX_LOG_LINES);
    DesktopSnapshot {
        profiles: Vec::new(),
        peers: Vec::new(),
        runtime: UiRuntime {
            phase: "failed".to_owned(),
            active_profile_id: None,
            started_at: None,
            retry_at: None,
            error: Some(error),
            peer_count: 0,
            peer_count_available: false,
            last_success_at: None,
            routes: 0,
            sent: 0,
            received: 0,
            daemon_version: "EasyTier Core 2.6.4（服务不可用）".to_owned(),
        },
        preferences: UiPreferences {
            auto_connect: false,
            service_at_boot: service_boot.configured,
            service_health: service_boot.health().to_owned(),
            theme,
        },
        logs,
    }
}

fn ui_phase(state: ServiceConnectionState) -> &'static str {
    match state {
        ServiceConnectionState::Disconnected => "disconnected",
        ServiceConnectionState::Connecting => "connecting",
        ServiceConnectionState::Connected => "connected",
        ServiceConnectionState::Recovering => "recovering",
        ServiceConnectionState::Failed => "failed",
    }
}

fn managed_instance_name(profile_id: &str) -> String {
    let identifier: String = profile_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(56)
        .collect();
    format!("vibe-{identifier}")
}

fn timestamp_to_iso(timestamp_ms: u64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(i64::try_from(timestamp_ms).ok()?)
        .map(|time| time.to_rfc3339())
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn export_file_name(profile_name: &str) -> String {
    let sanitized = profile_name
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let mut stem = sanitized.trim().trim_end_matches([' ', '.']);
    if stem
        .get(stem.len().saturating_sub(5)..)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(".toml"))
    {
        stem = stem[..stem.len() - 5].trim_end_matches([' ', '.']);
    }
    let truncated = stem.chars().take(96).collect::<String>();
    let truncated = truncated.trim_end_matches([' ', '.']);
    let mut stem = if truncated.is_empty() {
        "easytier-profile".to_owned()
    } else {
        truncated.to_owned()
    };
    let reserved_base = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        reserved_base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        stem.insert(0, '_');
    }
    format!("{stem}.toml")
}

/// Network errors are reduced to stable Chinese messages. The raw OS error
/// can include adapter details or addresses that should not enter the webview.
fn localize_bandwidth_error(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
            "内置 iperf3 测速组件缺失、损坏或无法启动，请重新安装应用。".to_owned()
        }
        std::io::ErrorKind::WouldBlock => {
            "目标节点正在执行其他 iperf3 测速，请稍后重试。".to_owned()
        }
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::TimedOut
        | std::io::ErrorKind::NotConnected
        | std::io::ErrorKind::AddrNotAvailable => {
            "无法连接目标节点的带宽测试服务，请确认对方已升级到相同版本，且防火墙允许节点间测速。"
                .to_owned()
        }
        std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::UnexpectedEof => {
            "节点间带宽测试连接中断，请检查网络稳定性后重试。".to_owned()
        }
        std::io::ErrorKind::InvalidData => {
            "iperf3 返回了无效结果，请确认双方客户端版本一致。".to_owned()
        }
        std::io::ErrorKind::InvalidInput => {
            "iperf3 测速参数无效，请检查活动档案的虚拟地址。".to_owned()
        }
        _ => "节点间带宽测试失败，请稍后重试。".to_owned(),
    }
}

/// Service and Core errors are deliberately reduced to Chinese user-facing
/// messages at the desktop boundary. Raw service text can contain paths,
/// command output, or TOML context that does not belong in the webview.
fn localize_service_error(error: &str) -> String {
    if let Some(message) = localize_core_config_validation_failure(error) {
        return message;
    }

    let normalized = error.to_ascii_lowercase();
    if normalized.contains("profile") && normalized.contains("does not exist") {
        "找不到指定的私有网络档案。".to_owned()
    } else if normalized.contains("no active profile") {
        "当前没有活动的私有网络档案。".to_owned()
    } else if normalized.contains("rejected the staged profile configuration")
        || normalized.contains("core config rejected")
    {
        "EasyTier Core 拒绝了本次配置，之前可用的档案和连接已保留。".to_owned()
    } else if normalized.contains("invalid easytier toml") {
        "导入的 EasyTier TOML 格式无效。".to_owned()
    } else if normalized.contains("unsupported easytier toml key") {
        "导入的 EasyTier TOML 包含当前版本不支持的配置项。".to_owned()
    } else if normalized.contains("invalid profile") {
        localize_profile_validation_error(&normalized)
    } else if normalized.contains("unavailable") || normalized.contains("named pipe") {
        "后台服务暂时不可用，请确认服务正在运行后重试。".to_owned()
    } else {
        "后台服务未能完成请求，请稍后重试。".to_owned()
    }
}

/// The service emits a deliberately fixed token vocabulary for Core config
/// validation. Never append the raw suffix here: older services or a corrupt
/// pipe response may contain paths, TOML, or a network secret.
fn localize_core_config_validation_failure(error: &str) -> Option<String> {
    let detail = error.strip_prefix(CORE_CONFIG_VALIDATION_ERROR_PREFIX)?;
    let (outcome, reason) = if detail == "validation timed out" {
        (
            "配置校验超时".to_owned(),
            "Core 未能在 10 秒内完成校验，请稍后重试".to_owned(),
        )
    } else if let Some(detail) = detail.strip_prefix("exit code ") {
        let (exit_code, reason) = detail.split_once("; reason=")?;
        let exit_code = exit_code.parse::<i32>().ok()?;
        (
            format!("退出码 {exit_code}"),
            core_config_reason_description(reason).to_owned(),
        )
    } else if let Some(reason) = detail.strip_prefix("terminated by the operating system; reason=")
    {
        (
            "校验进程异常结束".to_owned(),
            core_config_reason_description(reason).to_owned(),
        )
    } else {
        return Some(
            "EasyTier Core 未能完成配置校验，档案未保存；之前可用的档案和连接已保留。".to_owned(),
        );
    };

    Some(format!(
        "EasyTier Core 配置校验未通过（{outcome}）：{reason}。档案未保存，之前可用的档案和连接已保留。"
    ))
}

fn core_config_reason_description(reason: &str) -> &'static str {
    match reason {
        "network_identity" => "网络名称或网络密钥的格式不正确",
        "virtual_address" => "固定虚拟 IPv4/CIDR 设置不正确",
        "bootstrap_peer" => "Bootstrap Peer 地址或端口不正确",
        "core_option" => "Core 选项的取值或组合不正确",
        "toml_format" => "配置字段的类型或格式不正确",
        "file_access" => "Core 无法读取临时校验配置",
        _ => "配置未被 Core 接受，请检查网络名称、网络密钥、虚拟 IP、Bootstrap Peer 和 Core 选项",
    }
}

fn localize_profile_validation_error(normalized: &str) -> String {
    if normalized.contains("static ipv4") {
        if normalized.contains("must use address/prefix notation") {
            return "固定虚拟 IPv4 必须使用“IP/前缀长度”格式，例如 100.76.1.2/24。".to_owned();
        }
        if normalized.contains("invalid ipv4 address") {
            return "固定虚拟 IPv4 地址无效，请检查 IP 地址。".to_owned();
        }
        if normalized.contains("invalid prefix length") || normalized.contains("prefix length must")
        {
            return "固定虚拟 IPv4 的前缀长度无效，必须为 0 到 32。".to_owned();
        }
        return "固定虚拟 IPv4/CIDR 设置无效，请检查填写内容后重试。".to_owned();
    }

    if normalized.contains("address mode") {
        return "当前版本仅支持固定虚拟 IPv4/CIDR，不支持 DHCP。".to_owned();
    }

    if normalized.contains("peer") {
        if normalized.contains("at least one private bootstrap peer") {
            return "至少添加一个 Bootstrap Peer。".to_owned();
        }
        if normalized.contains("at most 8 peers") {
            return "Bootstrap Peer 最多添加 8 个。".to_owned();
        }
        if normalized.contains("duplicate bootstrap peer") {
            return "Bootstrap Peer 不能重复。".to_owned();
        }
        if normalized.contains("must include a host and port") {
            return "Bootstrap Peer 必须包含主机和端口，例如 tcp://seed.example:11010。".to_owned();
        }
        if normalized.contains("only tcp, udp, wg, ws, and wss are supported") {
            return "Bootstrap Peer 仅支持 tcp、udp、wg、ws 或 wss 协议。".to_owned();
        }
        if normalized.contains("only tcp, udp, ws, and wss are supported") {
            return "当前后台服务版本仅支持 tcp、udp、ws 或 wss 协议；升级服务后可使用 wg。"
                .to_owned();
        }
        if normalized.contains("must not contain credentials, query, or fragment") {
            return "Bootstrap Peer 不可包含账户、密码、查询参数或片段。".to_owned();
        }
        if normalized.contains("must be a valid uri") {
            return "Bootstrap Peer 地址格式无效。".to_owned();
        }
        return "Bootstrap Peer 设置无效，请检查填写内容后重试。".to_owned();
    }

    if normalized.contains("network secret") {
        return localize_text_field_error("网络密钥", normalized);
    }
    if normalized.contains("network name") {
        return localize_text_field_error("网络名称", normalized);
    }
    if normalized.contains("hostname") {
        return localize_text_field_error("设备名称", normalized);
    }
    if normalized.contains("profile name") || normalized.contains("invalid name") {
        return localize_text_field_error("档案名称", normalized);
    }
    if normalized.contains("flags.mtu") && normalized.contains("greater than zero") {
        return "MTU 必须大于 0。".to_owned();
    }
    if normalized.contains("flags.") {
        return "Core 选项无效，请检查取值或组合后重试。".to_owned();
    }
    "私有网络或 Core 选项无效，请检查填写内容后重试。".to_owned()
}

fn localize_text_field_error(field: &str, normalized: &str) -> String {
    if normalized.contains("must not be empty") {
        format!("{field} 不能为空。")
    } else if normalized.contains("must not exceed") {
        format!("{field} 超过允许长度。")
    } else if normalized.contains("must not contain control characters") {
        format!("{field} 不能包含控制字符。")
    } else {
        format!("{field} 无效，请检查填写内容后重试。")
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "preferences path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

#[cfg(windows)]
fn service_boot_status() -> ServiceBootStatus {
    let configured = sc_output(["qc", WINDOWS_SERVICE_NAME])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_ascii_uppercase())
        .is_some_and(|configuration| {
            configuration.contains("AUTO_START") && configuration.contains("DELAYED")
        });
    let running = sc_output(["query", WINDOWS_SERVICE_NAME])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_ascii_uppercase())
        .is_some_and(|state| state.contains("RUNNING") || state.contains("STATE              : 4"));
    ServiceBootStatus {
        configured,
        running,
    }
}

#[cfg(windows)]
fn sc_output<const N: usize>(arguments: [&str; N]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;

    // Tauri is a GUI process. Without CREATE_NO_WINDOW, each SCM status probe
    // can briefly create a visible sc.exe console window for the desktop user.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    Command::new("sc.exe")
        .args(arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

#[cfg(not(windows))]
fn service_boot_status() -> ServiceBootStatus {
    ServiceBootStatus {
        configured: false,
        running: false,
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let preferences_path = app
                .path()
                .app_data_dir()
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .join("desktop-preferences.json");
            let iperf3_executable = app
                .path()
                .resource_dir()
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .join("resources")
                .join("iperf3")
                .join("iperf3.exe");
            app.manage(NativeState::load(preferences_path, iperf3_executable));

            let show = MenuItem::with_id(app, "show", "显示 Vibe EasyTier", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出桌面端", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let icon = app.default_window_icon().cloned().ok_or_else(|| {
                std::io::Error::other("The bundled application icon is unavailable.")
            })?;
            let tray = TrayIconBuilder::with_id("vibe-easytier-tray")
                .icon(icon)
                .tooltip("Vibe EasyTier")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(&tray.app_handle());
                    }
                })
                .build(app)?;
            app.manage(tray);
            show_main_window(&app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            save_profile,
            update_profile_flags,
            import_profile_from_file,
            export_profile_toml,
            run_peer_bandwidth_test,
            delete_profile,
            select_active_profile,
            connect,
            disconnect,
            set_auto_connect,
            set_theme,
            clear_logs,
            minimize_window,
            toggle_maximize_window,
            hide_window,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Vibe EasyTier");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_file_name_is_safe_for_windows_and_keeps_a_toml_extension() {
        assert_eq!(export_file_name(" 办公网:华东? "), "办公网_华东_.toml");
        assert_eq!(export_file_name("..."), "easytier-profile.toml");
        assert_eq!(export_file_name("a/b\\c\n"), "a_b_c_.toml");
        assert_eq!(export_file_name("office.TOML"), "office.toml");
        assert_eq!(export_file_name("CON.txt"), "_CON.txt.toml");
    }

    #[test]
    fn bandwidth_errors_are_localized_without_rendering_os_details() {
        let error = std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connect to 100.64.0.8:29999 failed",
        );
        let message = localize_bandwidth_error(error);

        assert!(message.contains("目标节点"));
        assert!(!message.contains("100.64.0.8"));
        assert!(!message.contains("29999"));

        let missing = localize_bandwidth_error(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "C:\\secret-install-path\\iperf3.exe",
        ));
        assert!(missing.contains("iperf3"));
        assert!(!missing.contains("secret-install-path"));
    }

    #[test]
    fn bandwidth_test_binds_to_the_active_profiles_virtual_ipv4() {
        let mut status = ServiceStatus::stopped();
        status.active_profile_id = Some("office".to_owned());
        let profiles = vec![vibe_easytier_service::ProfileView {
            id: "office".to_owned(),
            name: "办公室".to_owned(),
            instance_name: "vibe-office".to_owned(),
            hostname: "workstation".to_owned(),
            network_name: "office-net".to_owned(),
            address_mode: AddressMode::Static {
                cidr: "100.76.1.2/24".to_owned(),
            },
            static_ipv4_cidr: Some("100.76.1.2/24".to_owned()),
            peers: Vec::new(),
            flags: EasyTierFlags::default(),
            auto_connect: true,
            secret_configured: true,
        }];

        assert_eq!(
            active_virtual_ipv4(&status, &profiles).unwrap(),
            Ipv4Addr::new(100, 76, 1, 2)
        );
    }

    #[test]
    fn multiline_info_log_keeps_the_level_from_its_first_line() {
        let message = concat!(
            "2026-08-01T15:29:23.000Z INFO easytier::connector: lookup failed\n",
            "    source: ResolveError {\n",
            "        kind: ProtoError,\n",
            "    }",
        );

        assert_eq!(core_log_level(message), "info");
        assert_eq!(
            core_log_level("2026-08-01T15:29:24.000Z WARN easytier: retry"),
            "warning"
        );
        assert_eq!(
            core_log_level("2026-08-01T15:29:25.000Z ERROR easytier: stopped"),
            "error"
        );
    }

    #[test]
    fn local_toml_import_accepts_utf8_bom_and_rejects_unsafe_input_sizes() {
        let decoded = decode_import_toml(b"\xef\xbb\xbfinstance_name = \"office\"\n".to_vec())
            .expect("UTF-8 TOML with a BOM should be accepted");
        assert_eq!(decoded, "instance_name = \"office\"\n");

        let oversized = decode_import_toml(vec![b'x'; MAX_IMPORT_TOML_BYTES + 1]).unwrap_err();
        assert!(oversized.contains("超过 1 MB"));

        let invalid_utf8 = decode_import_toml(vec![0xff]).unwrap_err();
        assert!(invalid_utf8.contains("UTF-8"));
    }

    #[test]
    fn ui_flags_round_trip_all_core_fields_without_losing_large_limits() {
        let mut service_flags = EasyTierFlags::default();
        service_flags.default_protocol = "udp".to_owned();
        service_flags.data_compress_algo = 2;
        service_flags.foreign_relay_bps_limit = 123_456_789_012_345;
        service_flags.instance_recv_bps_limit = u64::MAX;
        service_flags.enable_udp_broadcast_relay = true;

        let ui_flags = UiEasyTierFlags::from(service_flags.clone());
        let encoded = serde_json::to_value(&ui_flags).unwrap();
        let fields = encoded.as_object().unwrap();

        assert_eq!(
            fields.len(),
            vibe_easytier_service::EASYTIER_V2_6_4_FLAG_COUNT
        );
        assert_eq!(fields["dataCompressAlgo"], "zstd");
        assert_eq!(fields["foreignRelayBpsLimit"], "123456789012345");
        assert_eq!(fields["instanceRecvBpsLimit"], u64::MAX.to_string());
        assert!(fields.contains_key("enableUdpBroadcastRelay"));
        assert!(!fields.contains_key("networkSecret"));

        assert_eq!(EasyTierFlags::try_from(ui_flags).unwrap(), service_flags);
    }

    #[test]
    fn blank_bps_limit_means_unlimited_and_invalid_values_are_chinese_errors() {
        assert_eq!(parse_bps_limit("", "速率").unwrap(), u64::MAX);
        assert_eq!(parse_bps_limit(" 42 ", "速率").unwrap(), 42);
        assert!(parse_bps_limit("1.5", "速率")
            .unwrap_err()
            .contains("十进制"));
        assert!(parse_bps_limit("9223372036854775808", "速率")
            .unwrap_err()
            .contains("不能超过"));

        let mut ui_flags = UiEasyTierFlags::default();
        ui_flags.data_compress_algo = "gzip".to_owned();
        assert!(EasyTierFlags::try_from(ui_flags)
            .unwrap_err()
            .contains("数据压缩"));
    }

    #[test]
    fn profile_response_keeps_the_network_secret_out_of_the_webview() {
        let profile = vibe_easytier_service::ProfileView {
            id: "office".to_owned(),
            name: "办公室".to_owned(),
            instance_name: "vibe-office".to_owned(),
            hostname: "workstation".to_owned(),
            network_name: "office-net".to_owned(),
            address_mode: AddressMode::Static {
                cidr: "100.76.1.2/24".to_owned(),
            },
            static_ipv4_cidr: Some("100.76.1.2/24".to_owned()),
            peers: vec!["tcp://seed.example:11010".to_owned()],
            flags: EasyTierFlags::default(),
            auto_connect: true,
            secret_configured: true,
        };

        let encoded = serde_json::to_string(&UiProfile::from(profile)).unwrap();
        assert!(!encoded.contains("correct horse battery staple"));
        assert!(encoded.contains("\"networkSecret\":\"\""));
    }

    #[test]
    fn ui_peer_exposes_all_active_connection_protocols() {
        let peer = UiPeer::from(ConnectedPeer {
            id: "remote".to_owned(),
            hostname: "remote-node".to_owned(),
            ipv4: "10.44.0.3".to_owned(),
            cidr: Some("10.44.0.3/24".to_owned()),
            cost: Some("p2p".to_owned()),
            latency_ms: Some(8),
            rx_bytes: Some(12),
            tx_bytes: Some(34),
            protocols: vec!["tcp".to_owned(), "wg".to_owned()],
            tunnel_protocol: Some("tcp,wg".to_owned()),
            nat_type: None,
            version: Some("2.6.4".to_owned()),
        });

        assert_eq!(peer.protocols, vec!["tcp", "wg"]);
        let encoded = serde_json::to_value(peer).unwrap();
        assert_eq!(encoded["protocols"], serde_json::json!(["tcp", "wg"]));
        assert!(encoded.get("tunnelProtocol").is_none());
    }

    #[test]
    fn core_config_validation_errors_are_specific_chinese_and_safe_for_webview() {
        let cases = [
            (
                "easytier-core configuration validation failed: exit code 19; reason=network_identity",
                "网络名称或网络密钥",
            ),
            (
                "easytier-core configuration validation failed: exit code 3; reason=virtual_address",
                "固定虚拟 IPv4/CIDR",
            ),
            (
                "easytier-core configuration validation failed: exit code 1; reason=bootstrap_peer",
                "Bootstrap Peer 地址或端口",
            ),
            (
                "easytier-core configuration validation failed: exit code 101; reason=core_option",
                "Core 选项的取值或组合",
            ),
            (
                "easytier-core configuration validation failed: exit code 7; reason=unknown",
                "配置未被 Core 接受",
            ),
        ];

        for (service_error, expected_reason) in cases {
            let message = localize_service_error(service_error);
            assert!(message.contains("EasyTier Core 配置校验未通过"));
            assert!(message.contains(expected_reason));
            assert!(message.contains("档案未保存"));
            assert!(message.contains("之前可用的档案和连接已保留"));
        }

        let timeout = localize_service_error(
            "easytier-core configuration validation failed: validation timed out",
        );
        assert!(timeout.contains("配置校验超时"));
        assert!(timeout.contains("10 秒"));
    }

    #[test]
    fn core_config_validation_never_renders_untrusted_detail() {
        let secret = "correct-horse-battery-staple";
        let path = r"C:\ProgramData\VibeEasyTier\runtime\office.validate.toml";
        let error = format!(
            "{CORE_CONFIG_VALIDATION_ERROR_PREFIX}exit code 9; reason=network_identity; secret={secret}; path={path}"
        );
        let message = localize_service_error(&error);

        assert!(message.contains("配置未被 Core 接受"));
        assert!(!message.contains(secret));
        assert!(!message.contains(path));
    }

    #[test]
    fn local_profile_validation_keeps_a_useful_field_name() {
        let cases = [
            (
                "invalid profile: invalid static ipv4: must use address/prefix notation",
                "固定虚拟 IPv4 必须使用“IP/前缀长度”格式",
            ),
            (
                "invalid profile: invalid static ipv4: contains an invalid IPv4 address",
                "固定虚拟 IPv4 地址无效",
            ),
            (
                "invalid profile: invalid static ipv4: prefix length must be at most 32",
                "前缀长度无效",
            ),
            (
                "invalid profile: invalid peer: must include a host and port",
                "Bootstrap Peer 必须包含主机和端口",
            ),
            (
                "invalid profile: invalid peer: only tcp, udp, wg, ws, and wss are supported",
                "仅支持 tcp、udp、wg、ws 或 wss 协议",
            ),
            (
                "invalid profile: invalid peer: only tcp, udp, ws, and wss are supported",
                "当前后台服务版本仅支持 tcp、udp、ws 或 wss 协议；升级服务后可使用 wg",
            ),
            (
                "invalid profile: invalid peers: duplicate bootstrap peer",
                "Bootstrap Peer 不能重复",
            ),
            (
                "invalid profile: invalid peers: at most 8 peers are allowed",
                "Bootstrap Peer 最多添加 8 个",
            ),
        ];

        for (service_error, expected) in cases {
            assert!(
                localize_service_error(service_error).contains(expected),
                "expected {service_error:?} to describe {expected:?}"
            );
        }
    }
}
