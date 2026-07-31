//! Versioned, local-only RPC types shared by the desktop app and the service.
//!
//! Keep this surface intentionally narrow.  In particular, status and profile
//! listing messages never contain a network secret.

use serde::{Deserialize, Serialize};

use crate::profile::{AddressMode, EasyTierFlags, NetworkProfile};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileUpsert {
    pub profile: NetworkProfile,
    #[serde(default)]
    pub make_active: bool,
}

/// A requested durable connection state. `Connect` enables automatic
/// reconnection; `Disconnect` disables it before stopping the core process.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ConnectionIntent {
    Connect { profile_id: String },
    Disconnect { profile_id: Option<String> },
    SetAutoConnect { profile_id: String, enabled: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Recovering,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub protocol_version: u32,
    pub state: ServiceConnectionState,
    pub active_profile_id: Option<String>,
    pub auto_connect_profile_id: Option<String>,
    pub core_pid: Option<u32>,
    pub retry_at_unix_ms: Option<u64>,
    pub consecutive_failures: u32,
    /// Connected remote peers observed from a successful `easytier-cli peer
    /// list` call. It is zero when no peers are observed *or* when the
    /// optional observation is unavailable; inspect `peer_count_available` to
    /// distinguish those cases.
    pub peer_count: usize,
    #[serde(default)]
    pub peer_count_available: bool,
    /// Unix milliseconds of the most recent successful local core RPC health
    /// sample. This is control-plane health, not a promise that a remote peer
    /// was connected at that time.
    pub last_success_unix_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl ServiceStatus {
    pub fn stopped() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            state: ServiceConnectionState::Disconnected,
            active_profile_id: None,
            auto_connect_profile_id: None,
            core_pid: None,
            retry_at_unix_ms: None,
            consecutive_failures: 0,
            peer_count: 0,
            peer_count_available: false,
            last_success_unix_ms: None,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub network_name: String,
    pub peer_count: usize,
    pub auto_connect: bool,
}

impl From<&NetworkProfile> for ProfileSummary {
    fn from(profile: &NetworkProfile) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            hostname: profile.hostname.clone(),
            network_name: profile.network_name.clone(),
            peer_count: profile.peers.len(),
            auto_connect: profile.auto_connect,
        }
    }
}

/// Complete renderable profile data. This deliberately excludes the network
/// secret, but preserves the address and bootstrap configuration the desktop
/// needs to display after it reconnects to the service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub instance_name: String,
    pub hostname: String,
    pub network_name: String,
    pub address_mode: AddressMode,
    pub static_ipv4_cidr: Option<String>,
    pub peers: Vec<String>,
    /// Complete typed EasyTier v2.6.4 `[flags]` data. It contains no secret
    /// material and is returned so the desktop can render the settings form.
    #[serde(default)]
    pub flags: EasyTierFlags,
    pub auto_connect: bool,
    pub secret_configured: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceLogLine {
    pub source: String,
    pub line: String,
}

/// A sanitized peer row obtained by the service from the bundled EasyTier
/// CLI. It intentionally omits every configuration or management endpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectedPeer {
    pub id: String,
    pub hostname: String,
    pub ipv4: String,
    pub cidr: Option<String>,
    pub cost: Option<String>,
    pub latency_ms: Option<u32>,
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
    /// Every active direct tunnel transport reported by EasyTier for this
    /// remote peer. EasyTier v2.6.4 emits a comma-separated `tunnel_proto`
    /// value when more than one connection is alive; the service exposes the
    /// normalized values as an ordered, de-duplicated list for the desktop.
    #[serde(default)]
    pub protocols: Vec<String>,
    /// Compatibility mirror of `protocols` for older local consumers. New
    /// clients should use `protocols`, because a peer may have more than one
    /// active transport.
    #[serde(default)]
    pub tunnel_protocol: Option<String>,
    pub nat_type: Option<String>,
    pub version: Option<String>,
}

impl From<&NetworkProfile> for ProfileView {
    fn from(profile: &NetworkProfile) -> Self {
        let static_ipv4_cidr = match &profile.address_mode {
            AddressMode::Dhcp => None,
            AddressMode::Static { cidr } => Some(cidr.clone()),
        };
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            instance_name: profile.instance_name.clone(),
            hostname: profile.hostname.clone(),
            network_name: profile.network_name.clone(),
            address_mode: profile.address_mode.clone(),
            static_ipv4_cidr,
            peers: profile.peers.clone(),
            flags: profile.flags.clone(),
            auto_connect: profile.auto_connect,
            secret_configured: !profile.network_secret.expose().is_empty(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum RpcCommand {
    Ping,
    GetStatus,
    ListProfiles,
    ListPeers,
    UpsertProfile(ProfileUpsert),
    /// Changes only an existing profile's typed flag set. The service retains
    /// its network secret, validates the staged core TOML, and commits
    /// atomically only after that validation succeeds.
    UpdateProfileFlags {
        profile_id: String,
        flags: EasyTierFlags,
    },
    ImportProfile {
        toml: String,
        make_active: bool,
    },
    DeleteProfile {
        profile_id: String,
    },
    SetActiveProfile {
        profile_id: String,
    },
    SetConnectionIntent {
        intent: ConnectionIntent,
    },
    TailLogs {
        limit: usize,
    },
    ClearLogs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RpcRequest {
    #[serde(default = "default_protocol_version")]
    pub protocol_version: u32,
    pub request_id: u64,
    #[serde(flatten)]
    pub command: RpcCommand,
}

impl RpcRequest {
    pub fn new(request_id: u64, command: RpcCommand) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            command,
        }
    }

    pub fn is_compatible(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RpcResult {
    Pong,
    Status(ServiceStatus),
    Profiles(Vec<ProfileView>),
    Peers(Vec<ConnectedPeer>),
    ProfileSaved(ProfileView),
    ProfileDeleted { profile_id: String },
    ActiveProfileSelected(ServiceStatus),
    IntentApplied(ServiceStatus),
    Logs(Vec<ServiceLogLine>),
    LogsCleared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcErrorCode {
    InvalidRequest,
    InvalidProfile,
    NotFound,
    Conflict,
    UnsupportedVersion,
    Unavailable,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: RpcErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RpcResponse {
    pub protocol_version: u32,
    pub request_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RpcResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(request_id: u64, result: RpcResult) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(request_id: u64, code: RpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

fn default_protocol_version() -> u32 {
    PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_uses_a_stable_method_tag() {
        let request = RpcRequest::new(7, RpcCommand::GetStatus);
        let value = serde_json::to_value(&request).unwrap();

        assert_eq!(value["method"], "get_status");
        assert_eq!(
            serde_json::from_value::<RpcRequest>(value).unwrap(),
            request
        );
    }

    #[test]
    fn public_profile_view_does_not_contain_network_secret_material() {
        let encoded = serde_json::to_string(&ProfileView {
            id: "home".to_owned(),
            name: "Home".to_owned(),
            instance_name: "home".to_owned(),
            hostname: "laptop".to_owned(),
            network_name: "private-home".to_owned(),
            address_mode: AddressMode::Static {
                cidr: "10.44.0.2/24".to_owned(),
            },
            static_ipv4_cidr: Some("10.44.0.2/24".to_owned()),
            peers: vec!["tcp://seed.example.net:11010".to_owned()],
            flags: EasyTierFlags::default(),
            auto_connect: true,
            secret_configured: true,
        })
        .unwrap();

        assert!(!encoded.contains("correct horse battery staple"));
    }

    #[test]
    fn connected_peer_protocols_are_a_stable_array_in_ipc_json() {
        let peer = ConnectedPeer {
            id: "remote".to_owned(),
            hostname: "remote-node".to_owned(),
            ipv4: "10.44.0.3".to_owned(),
            cidr: Some("10.44.0.3/24".to_owned()),
            cost: Some("p2p".to_owned()),
            latency_ms: Some(12),
            rx_bytes: Some(123),
            tx_bytes: Some(456),
            protocols: vec!["tcp".to_owned(), "wg".to_owned()],
            tunnel_protocol: Some("tcp,wg".to_owned()),
            nat_type: Some("FullCone".to_owned()),
            version: Some("2.6.4".to_owned()),
        };

        let json = serde_json::to_value(peer).unwrap();
        assert_eq!(json["protocols"], serde_json::json!(["tcp", "wg"]));
        assert_eq!(json["tunnel_protocol"], "tcp,wg");
    }
}
