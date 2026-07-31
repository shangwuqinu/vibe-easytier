use std::{collections::BTreeSet, fmt, net::Ipv4Addr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

pub const CORE_RPC_PORTAL: &str = "127.0.0.1:15888";
pub const CORE_RPC_PORTAL_WHITELIST: &str = "127.0.0.1/32";
const MAX_PEERS: usize = 8;

/// EasyTier v2.6.4 exposes 41 fields in `common.FlagsInConfig`.  Keep this
/// number next to the typed schema so additions in a future bundled core are
/// deliberate rather than silently discarded during TOML import.
pub const EASYTIER_V2_6_4_FLAG_COUNT: usize = 41;

/// Exact `[flags]` keys supported by the bundled EasyTier v2.6.4 core.
///
/// `quic_listen_port` is deprecated upstream but is intentionally retained
/// for compatibility with existing configuration files.
pub const EASYTIER_V2_6_4_FLAG_KEYS: [&str; EASYTIER_V2_6_4_FLAG_COUNT] = [
    "default_protocol",
    "dev_name",
    "enable_encryption",
    "enable_ipv6",
    "mtu",
    "latency_first",
    "enable_exit_node",
    "no_tun",
    "use_smoltcp",
    "relay_network_whitelist",
    "disable_p2p",
    "relay_all_peer_rpc",
    "disable_udp_hole_punching",
    "multi_thread",
    "data_compress_algo",
    "bind_device",
    "enable_kcp_proxy",
    "disable_kcp_input",
    "disable_relay_kcp",
    "proxy_forward_by_system",
    "accept_dns",
    "private_mode",
    "enable_quic_proxy",
    "disable_quic_input",
    "disable_relay_quic",
    "quic_listen_port",
    "foreign_relay_bps_limit",
    "multi_thread_count",
    "enable_relay_foreign_network_kcp",
    "enable_relay_foreign_network_quic",
    "encryption_algorithm",
    "disable_sym_hole_punching",
    "tld_dns_zone",
    "p2p_only",
    "disable_tcp_hole_punching",
    "lazy_p2p",
    "need_p2p",
    "instance_recv_bps_limit",
    "disable_upnp",
    "disable_relay_data",
    "enable_udp_broadcast_relay",
];

/// Typed EasyTier v2.6.4 `[flags]` configuration.
///
/// The two BPS limits use `u64::MAX` as the in-memory unlimited sentinel, as
/// EasyTier does. TOML has signed 64-bit integers, so the renderer omits those
/// two fields when they hold the sentinel and lets core apply its unlimited
/// default. This avoids producing an invalid TOML integer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EasyTierFlags {
    pub default_protocol: String,
    pub dev_name: String,
    pub enable_encryption: bool,
    pub enable_ipv6: bool,
    pub mtu: u32,
    pub latency_first: bool,
    pub enable_exit_node: bool,
    pub no_tun: bool,
    pub use_smoltcp: bool,
    pub relay_network_whitelist: String,
    pub disable_p2p: bool,
    pub relay_all_peer_rpc: bool,
    pub disable_udp_hole_punching: bool,
    pub multi_thread: bool,
    /// EasyTier protobuf `CompressionAlgoPb`: `1` is none and `2` is zstd.
    pub data_compress_algo: i32,
    pub bind_device: bool,
    pub enable_kcp_proxy: bool,
    pub disable_kcp_input: bool,
    pub disable_relay_kcp: bool,
    pub proxy_forward_by_system: bool,
    pub accept_dns: bool,
    pub private_mode: bool,
    pub enable_quic_proxy: bool,
    pub disable_quic_input: bool,
    pub disable_relay_quic: bool,
    /// Deprecated upstream but still deserialized by EasyTier v2.6.4.
    pub quic_listen_port: u32,
    pub foreign_relay_bps_limit: u64,
    pub multi_thread_count: u32,
    pub enable_relay_foreign_network_kcp: bool,
    pub enable_relay_foreign_network_quic: bool,
    pub encryption_algorithm: String,
    pub disable_sym_hole_punching: bool,
    pub tld_dns_zone: String,
    pub p2p_only: bool,
    pub disable_tcp_hole_punching: bool,
    pub lazy_p2p: bool,
    pub need_p2p: bool,
    pub instance_recv_bps_limit: u64,
    pub disable_upnp: bool,
    pub disable_relay_data: bool,
    pub enable_udp_broadcast_relay: bool,
}

impl Default for EasyTierFlags {
    fn default() -> Self {
        Self {
            // Matches EasyTier v2.6.4's `gen_default_flags`, except this
            // client intentionally starts private mode enabled. Existing
            // profiles therefore retain their prior private/encrypted setup.
            default_protocol: "tcp".to_owned(),
            dev_name: String::new(),
            enable_encryption: true,
            enable_ipv6: true,
            mtu: 1380,
            latency_first: false,
            enable_exit_node: false,
            no_tun: false,
            use_smoltcp: false,
            relay_network_whitelist: "*".to_owned(),
            disable_p2p: false,
            relay_all_peer_rpc: false,
            disable_udp_hole_punching: false,
            multi_thread: true,
            data_compress_algo: 1,
            bind_device: true,
            enable_kcp_proxy: false,
            disable_kcp_input: false,
            disable_relay_kcp: false,
            proxy_forward_by_system: false,
            accept_dns: false,
            private_mode: true,
            enable_quic_proxy: false,
            disable_quic_input: false,
            disable_relay_quic: false,
            quic_listen_port: u32::MAX,
            foreign_relay_bps_limit: u64::MAX,
            multi_thread_count: 2,
            enable_relay_foreign_network_kcp: false,
            enable_relay_foreign_network_quic: false,
            encryption_algorithm: "aes-gcm".to_owned(),
            disable_sym_hole_punching: false,
            tld_dns_zone: "et.net.".to_owned(),
            p2p_only: false,
            disable_tcp_hole_punching: false,
            lazy_p2p: false,
            need_p2p: false,
            instance_recv_bps_limit: u64::MAX,
            disable_upnp: false,
            disable_relay_data: false,
            enable_udp_broadcast_relay: false,
        }
    }
}

/// A serializable secret that deliberately redacts itself in diagnostics.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([redacted])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AddressMode {
    Dhcp,
    Static { cidr: String },
}

/// Service-owned EasyTier profile data. Network identity and runtime placement
/// remain service-controlled; all flags offered by the bundled core are stored
/// as typed profile data.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkProfile {
    pub id: String,
    pub name: String,
    pub instance_name: String,
    pub hostname: String,
    pub network_name: String,
    pub network_secret: SecretString,
    pub address_mode: AddressMode,
    pub peers: Vec<String>,
    /// Defaults allow older encrypted state files to migrate without changing
    /// their existing private/encrypted behavior.
    #[serde(default)]
    pub flags: EasyTierFlags,
    #[serde(default)]
    pub auto_connect: bool,
}

impl fmt::Debug for NetworkProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkProfile")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("instance_name", &self.instance_name)
            .field("hostname", &self.hostname)
            .field("network_name", &self.network_name)
            .field("network_secret", &self.network_secret)
            .field("address_mode", &self.address_mode)
            .field("peers", &self.peers)
            .field("flags", &self.flags)
            .field("auto_connect", &self.auto_connect)
            .finish()
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProfileError {
    #[error("invalid {field}: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("unsupported EasyTier TOML key: {path}")]
    DisallowedTomlKey { path: String },
    #[error("invalid EasyTier TOML: {0}")]
    InvalidToml(String),
}

impl NetworkProfile {
    /// Resolves an empty device-name field to this machine's Windows computer
    /// name before the profile is validated or persisted.  Explicit names are
    /// left untouched so a user can deliberately give a device a network name
    /// that differs from the local computer name.
    pub fn apply_default_hostname(&mut self) {
        self.apply_default_hostname_with(local_computer_name);
    }

    fn apply_default_hostname_with(&mut self, computer_name: impl FnOnce() -> String) {
        if self.hostname.trim().is_empty() {
            self.hostname = computer_name();
        }
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        validate_identifier("profile id", &self.id)?;
        validate_identifier("instance name", &self.instance_name)?;
        validate_text("profile name", &self.name, 96)?;
        validate_text("hostname", &self.hostname, 63)?;
        validate_text("network name", &self.network_name, 128)?;
        validate_text("network secret", self.network_secret.expose(), 512)?;

        match &self.address_mode {
            // Version one intentionally manages one stable virtual IPv4
            // address per profile.  Retain this enum variant only so an
            // older serialized request gets a clear validation error rather
            // than silently enabling a different core mode.
            AddressMode::Dhcp => {
                return Err(invalid(
                    "address mode",
                    "v1 requires a fixed virtual IPv4/CIDR",
                ));
            }
            AddressMode::Static { cidr } => validate_ipv4_cidr(cidr)?,
        }

        if self.peers.is_empty() {
            return Err(invalid(
                "peers",
                "at least one private bootstrap peer is required",
            ));
        }
        if self.peers.len() > MAX_PEERS {
            return Err(invalid(
                "peers",
                format!("at most {MAX_PEERS} peers are allowed"),
            ));
        }

        let mut unique = BTreeSet::new();
        for peer in &self.peers {
            validate_peer_uri(peer)?;
            if !unique.insert(peer) {
                return Err(invalid("peers", "duplicate bootstrap peer"));
            }
        }
        self.flags.validate()?;
        Ok(())
    }

    /// Generates only the slim, service-controlled EasyTier configuration.
    /// The runtime TOML is written beneath the service-owned ProgramData
    /// directory, so the secret stays out of the core process command line.
    pub fn render_core_toml(&self) -> Result<String, ProfileError> {
        self.validate()?;

        let ipv4 = match &self.address_mode {
            AddressMode::Static { cidr } => cidr.as_str(),
            AddressMode::Dhcp => {
                return Err(invalid(
                    "address mode",
                    "v1 requires a fixed virtual IPv4/CIDR",
                ));
            }
        };
        let document = CoreToml {
            instance_name: &self.instance_name,
            hostname: &self.hostname,
            ipv4,
            peer: self.peers.iter().map(|uri| PeerToml { uri }).collect(),
            network_identity: NetworkIdentityToml {
                network_name: &self.network_name,
                network_secret: self.network_secret.expose(),
            },
            flags: CoreFlagsToml::from(&self.flags),
        };

        toml::to_string_pretty(&document)
            .map_err(|error| ProfileError::InvalidToml(error.to_string()))
    }

    /// Imports only the root-level fields owned by this client, while accepting
    /// every `[flags]` key supported by the bundled EasyTier v2.6.4 core.
    /// Listeners and other topology-changing root fields stay service-managed.
    pub fn from_whitelisted_toml(input: &str) -> Result<Self, ProfileError> {
        let root: toml::Value =
            toml::from_str(input).map_err(|error| ProfileError::InvalidToml(error.to_string()))?;
        let root = root
            .as_table()
            .ok_or_else(|| ProfileError::InvalidToml("root must be a TOML table".to_owned()))?;

        ensure_allowed_keys(
            root,
            "",
            &[
                "instance_name",
                "hostname",
                "ipv4",
                "peer",
                "network_identity",
                "flags",
            ],
        )?;
        let instance_name = required_string(root, "instance_name")?;
        let hostname = required_string(root, "hostname")?;
        let address_mode = AddressMode::Static {
            cidr: required_string(root, "ipv4")?,
        };

        let peers = parse_peers(root)?;
        let identity = required_table(root, "network_identity")?;
        ensure_allowed_keys(
            identity,
            "network_identity",
            &["network_name", "network_secret"],
        )?;
        let network_name = required_string(identity, "network_name")?;
        let network_secret = SecretString::new(required_string(identity, "network_secret")?);

        let flags = match root.get("flags") {
            None => EasyTierFlags::default(),
            Some(value) => {
                let table = value
                    .as_table()
                    .ok_or_else(|| invalid("flags", "must be a table"))?;
                EasyTierFlags::from_toml_table(table)?
            }
        };

        let mut profile = Self {
            id: instance_name.clone(),
            name: instance_name.clone(),
            instance_name,
            hostname,
            network_name,
            network_secret,
            address_mode,
            peers,
            flags,
            auto_connect: false,
        };
        profile.apply_default_hostname();
        profile.validate()?;
        Ok(profile)
    }
}

/// Returns the Windows computer name even when the service is running as
/// LocalSystem.  The environment-variable fallback also makes configuration
/// handling useful in non-Windows tests and development hosts.
fn local_computer_name() -> String {
    #[cfg(windows)]
    if let Some(name) = windows_computer_name() {
        return name;
    }

    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_owned())
        .find(|value| is_usable_hostname(value))
        .unwrap_or_else(|| "vibe-easytier".to_owned())
}

#[cfg(windows)]
fn windows_computer_name() -> Option<String> {
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;

    // Windows computer names are substantially shorter in normal use, but a
    // 256 UTF-16-code-unit buffer avoids relying on a legacy NetBIOS limit.
    let mut buffer = vec![0u16; 256];
    let mut length = buffer.len() as u32;
    let succeeded = unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut length) } != 0;
    if !succeeded {
        return None;
    }

    String::from_utf16(&buffer[..length as usize])
        .ok()
        .filter(|value| is_usable_hostname(value))
}

fn is_usable_hostname(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 63 && !value.chars().any(char::is_control)
}

#[derive(Serialize)]
struct CoreToml<'a> {
    instance_name: &'a str,
    hostname: &'a str,
    ipv4: &'a str,
    peer: Vec<PeerToml<'a>>,
    network_identity: NetworkIdentityToml<'a>,
    flags: CoreFlagsToml<'a>,
}

#[derive(Serialize)]
struct PeerToml<'a> {
    uri: &'a str,
}

#[derive(Serialize)]
struct NetworkIdentityToml<'a> {
    network_name: &'a str,
    network_secret: &'a str,
}

#[derive(Serialize)]
struct CoreFlagsToml<'a> {
    default_protocol: &'a str,
    dev_name: &'a str,
    enable_encryption: bool,
    enable_ipv6: bool,
    mtu: u32,
    latency_first: bool,
    enable_exit_node: bool,
    no_tun: bool,
    use_smoltcp: bool,
    relay_network_whitelist: &'a str,
    disable_p2p: bool,
    relay_all_peer_rpc: bool,
    disable_udp_hole_punching: bool,
    multi_thread: bool,
    data_compress_algo: i32,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    quic_listen_port: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreign_relay_bps_limit: Option<u64>,
    multi_thread_count: u32,
    enable_relay_foreign_network_kcp: bool,
    enable_relay_foreign_network_quic: bool,
    encryption_algorithm: &'a str,
    disable_sym_hole_punching: bool,
    tld_dns_zone: &'a str,
    p2p_only: bool,
    disable_tcp_hole_punching: bool,
    lazy_p2p: bool,
    need_p2p: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_recv_bps_limit: Option<u64>,
    disable_upnp: bool,
    disable_relay_data: bool,
    enable_udp_broadcast_relay: bool,
}

impl<'a> From<&'a EasyTierFlags> for CoreFlagsToml<'a> {
    fn from(flags: &'a EasyTierFlags) -> Self {
        Self {
            default_protocol: &flags.default_protocol,
            dev_name: &flags.dev_name,
            enable_encryption: flags.enable_encryption,
            enable_ipv6: flags.enable_ipv6,
            mtu: flags.mtu,
            latency_first: flags.latency_first,
            enable_exit_node: flags.enable_exit_node,
            no_tun: flags.no_tun,
            use_smoltcp: flags.use_smoltcp,
            relay_network_whitelist: &flags.relay_network_whitelist,
            disable_p2p: flags.disable_p2p,
            relay_all_peer_rpc: flags.relay_all_peer_rpc,
            disable_udp_hole_punching: flags.disable_udp_hole_punching,
            multi_thread: flags.multi_thread,
            data_compress_algo: flags.data_compress_algo,
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
            quic_listen_port: (flags.quic_listen_port != u32::MAX)
                .then_some(flags.quic_listen_port),
            foreign_relay_bps_limit: (flags.foreign_relay_bps_limit != u64::MAX)
                .then_some(flags.foreign_relay_bps_limit),
            multi_thread_count: flags.multi_thread_count,
            enable_relay_foreign_network_kcp: flags.enable_relay_foreign_network_kcp,
            enable_relay_foreign_network_quic: flags.enable_relay_foreign_network_quic,
            encryption_algorithm: &flags.encryption_algorithm,
            disable_sym_hole_punching: flags.disable_sym_hole_punching,
            tld_dns_zone: &flags.tld_dns_zone,
            p2p_only: flags.p2p_only,
            disable_tcp_hole_punching: flags.disable_tcp_hole_punching,
            lazy_p2p: flags.lazy_p2p,
            need_p2p: flags.need_p2p,
            instance_recv_bps_limit: (flags.instance_recv_bps_limit != u64::MAX)
                .then_some(flags.instance_recv_bps_limit),
            disable_upnp: flags.disable_upnp,
            disable_relay_data: flags.disable_relay_data,
            enable_udp_broadcast_relay: flags.enable_udp_broadcast_relay,
        }
    }
}

impl EasyTierFlags {
    fn from_toml_table(table: &toml::map::Map<String, toml::Value>) -> Result<Self, ProfileError> {
        ensure_allowed_keys(table, "flags", &EASYTIER_V2_6_4_FLAG_KEYS)?;
        let mut flags = Self::default();

        macro_rules! set_bool {
            ($field:ident, $key:literal) => {
                if let Some(value) = optional_flag_bool(table, $key, concat!("flags.", $key))? {
                    flags.$field = value;
                }
            };
        }
        macro_rules! set_string {
            ($field:ident, $key:literal) => {
                if let Some(value) = optional_flag_string(table, $key, concat!("flags.", $key))? {
                    flags.$field = value;
                }
            };
        }
        macro_rules! set_u32 {
            ($field:ident, $key:literal) => {
                if let Some(value) = optional_flag_u32(table, $key, concat!("flags.", $key))? {
                    flags.$field = value;
                }
            };
        }
        macro_rules! set_u64 {
            ($field:ident, $key:literal) => {
                if let Some(value) = optional_flag_u64(table, $key, concat!("flags.", $key))? {
                    flags.$field = value;
                }
            };
        }

        set_string!(default_protocol, "default_protocol");
        set_string!(dev_name, "dev_name");
        set_bool!(enable_encryption, "enable_encryption");
        set_bool!(enable_ipv6, "enable_ipv6");
        set_u32!(mtu, "mtu");
        set_bool!(latency_first, "latency_first");
        set_bool!(enable_exit_node, "enable_exit_node");
        set_bool!(no_tun, "no_tun");
        set_bool!(use_smoltcp, "use_smoltcp");
        set_string!(relay_network_whitelist, "relay_network_whitelist");
        set_bool!(disable_p2p, "disable_p2p");
        set_bool!(relay_all_peer_rpc, "relay_all_peer_rpc");
        set_bool!(disable_udp_hole_punching, "disable_udp_hole_punching");
        set_bool!(multi_thread, "multi_thread");
        if let Some(value) = optional_compression_algo(table)? {
            flags.data_compress_algo = value;
        }
        set_bool!(bind_device, "bind_device");
        set_bool!(enable_kcp_proxy, "enable_kcp_proxy");
        set_bool!(disable_kcp_input, "disable_kcp_input");
        set_bool!(disable_relay_kcp, "disable_relay_kcp");
        set_bool!(proxy_forward_by_system, "proxy_forward_by_system");
        set_bool!(accept_dns, "accept_dns");
        set_bool!(private_mode, "private_mode");
        set_bool!(enable_quic_proxy, "enable_quic_proxy");
        set_bool!(disable_quic_input, "disable_quic_input");
        set_bool!(disable_relay_quic, "disable_relay_quic");
        set_u32!(quic_listen_port, "quic_listen_port");
        set_u64!(foreign_relay_bps_limit, "foreign_relay_bps_limit");
        set_u32!(multi_thread_count, "multi_thread_count");
        set_bool!(
            enable_relay_foreign_network_kcp,
            "enable_relay_foreign_network_kcp"
        );
        set_bool!(
            enable_relay_foreign_network_quic,
            "enable_relay_foreign_network_quic"
        );
        set_string!(encryption_algorithm, "encryption_algorithm");
        set_bool!(disable_sym_hole_punching, "disable_sym_hole_punching");
        set_string!(tld_dns_zone, "tld_dns_zone");
        set_bool!(p2p_only, "p2p_only");
        set_bool!(disable_tcp_hole_punching, "disable_tcp_hole_punching");
        set_bool!(lazy_p2p, "lazy_p2p");
        set_bool!(need_p2p, "need_p2p");
        set_u64!(instance_recv_bps_limit, "instance_recv_bps_limit");
        set_bool!(disable_upnp, "disable_upnp");
        set_bool!(disable_relay_data, "disable_relay_data");
        set_bool!(enable_udp_broadcast_relay, "enable_udp_broadcast_relay");

        flags.validate()?;
        Ok(flags)
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        validate_flag_text("flags.default_protocol", &self.default_protocol, 32, false)?;
        validate_flag_text("flags.dev_name", &self.dev_name, 128, true)?;
        validate_flag_text(
            "flags.relay_network_whitelist",
            &self.relay_network_whitelist,
            4096,
            true,
        )?;
        if self.mtu == 0 {
            return Err(invalid("flags.mtu", "must be greater than zero"));
        }
        if !matches!(self.data_compress_algo, 1 | 2) {
            return Err(invalid(
                "flags.data_compress_algo",
                "must be 1 (none) or 2 (zstd)",
            ));
        }
        if self.multi_thread_count == 0 {
            return Err(invalid(
                "flags.multi_thread_count",
                "must be greater than zero",
            ));
        }
        if self.quic_listen_port != u32::MAX && self.quic_listen_port > u16::MAX as u32 {
            return Err(invalid(
                "flags.quic_listen_port",
                "must be a TCP/UDP port (0-65535) or the unlimited sentinel",
            ));
        }
        validate_toml_bps_limit(
            "flags.foreign_relay_bps_limit",
            self.foreign_relay_bps_limit,
        )?;
        validate_toml_bps_limit(
            "flags.instance_recv_bps_limit",
            self.instance_recv_bps_limit,
        )?;
        if !matches!(
            self.encryption_algorithm.as_str(),
            "" | "xor" | "aes-gcm" | "aes-256-gcm" | "chacha20"
        ) {
            return Err(invalid(
                "flags.encryption_algorithm",
                "must be xor, aes-gcm, aes-256-gcm, chacha20, or empty for the core default",
            ));
        }
        validate_flag_text("flags.tld_dns_zone", &self.tld_dns_zone, 253, false)?;
        Ok(())
    }
}

fn invalid(field: &'static str, reason: impl Into<String>) -> ProfileError {
    ProfileError::InvalidField {
        field,
        reason: reason.into(),
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ProfileError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(invalid(
            field,
            "use 1-64 ASCII letters, numbers, hyphens, or underscores",
        ))
    }
}

fn validate_text(field: &'static str, value: &str, max_chars: usize) -> Result<(), ProfileError> {
    if value.trim().is_empty() {
        return Err(invalid(field, "must not be empty"));
    }
    if value.chars().count() > max_chars {
        return Err(invalid(
            field,
            format!("must not exceed {max_chars} characters"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid(field, "must not contain control characters"));
    }
    Ok(())
}

fn validate_ipv4_cidr(cidr: &str) -> Result<(), ProfileError> {
    let (address, prefix) = cidr
        .split_once('/')
        .ok_or_else(|| invalid("static ipv4", "must use address/prefix notation"))?;
    address
        .parse::<Ipv4Addr>()
        .map_err(|_| invalid("static ipv4", "contains an invalid IPv4 address"))?;
    let prefix = prefix
        .parse::<u8>()
        .map_err(|_| invalid("static ipv4", "contains an invalid prefix length"))?;
    if prefix > 32 {
        return Err(invalid("static ipv4", "prefix length must be at most 32"));
    }
    Ok(())
}

fn validate_peer_uri(value: &str) -> Result<(), ProfileError> {
    let url = Url::parse(value).map_err(|_| invalid("peer", "must be a valid URI"))?;
    if !matches!(url.scheme(), "tcp" | "udp" | "wg" | "ws" | "wss") {
        return Err(invalid(
            "peer",
            "only tcp, udp, wg, ws, and wss are supported",
        ));
    }
    // `url` canonicalizes a scheme-default port away, so
    // `wss://seed.example.net:443` reports no port even though the user
    // explicitly supplied one. Preserve the requirement for a literal port
    // by inspecting the already-validated URI source as well.
    if url.host_str().is_none() || (url.port().is_none() && !has_explicit_port(value)) {
        return Err(invalid("peer", "must include a host and port"));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "peer",
            "must not contain credentials, query, or fragment",
        ));
    }
    Ok(())
}

fn has_explicit_port(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority_end = remainder
        .find(|character| matches!(character, '/' | '?' | '#'))
        .unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host_and_port)| host_and_port);

    let port = if let Some(bracketed_host) = host_and_port.strip_prefix('[') {
        let Some(closing_bracket) = bracketed_host.find(']') else {
            return false;
        };
        bracketed_host[closing_bracket + 1..].strip_prefix(':')
    } else {
        host_and_port
            .rsplit_once(':')
            .and_then(|(host, port)| (!host.is_empty()).then_some(port))
    };

    port.is_some_and(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
}

fn ensure_allowed_keys(
    table: &toml::map::Map<String, toml::Value>,
    prefix: &str,
    allowed: &[&str],
) -> Result<(), ProfileError> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            return Err(ProfileError::DisallowedTomlKey { path });
        }
    }
    Ok(())
}

fn required_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &'static str,
) -> Result<String, ProfileError> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid(key, "is required and must be a string"))
}

fn validate_flag_text(
    field: &'static str,
    value: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), ProfileError> {
    if !allow_empty && value.trim().is_empty() {
        return Err(invalid(field, "must not be empty"));
    }
    if value.chars().count() > max_chars {
        return Err(invalid(
            field,
            format!("must not exceed {max_chars} characters"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid(field, "must not contain control characters"));
    }
    Ok(())
}

fn validate_toml_bps_limit(field: &'static str, value: u64) -> Result<(), ProfileError> {
    if value != u64::MAX && value > i64::MAX as u64 {
        return Err(invalid(
            field,
            "must not exceed TOML's signed 64-bit integer range unless unlimited",
        ));
    }
    Ok(())
}

fn optional_flag_bool(
    table: &toml::map::Map<String, toml::Value>,
    key: &'static str,
    field: &'static str,
) -> Result<Option<bool>, ProfileError> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| invalid(field, "must be a boolean")),
    }
}

fn optional_flag_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &'static str,
    field: &'static str,
) -> Result<Option<String>, ProfileError> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(ToOwned::to_owned)
            .map(Some)
            .ok_or_else(|| invalid(field, "must be a string")),
    }
}

fn optional_flag_integer(
    table: &toml::map::Map<String, toml::Value>,
    key: &'static str,
    field: &'static str,
) -> Result<Option<i64>, ProfileError> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_integer()
            .ok_or_else(|| invalid(field, "must be an integer"))
            .map(Some),
    }
}

fn optional_flag_u32(
    table: &toml::map::Map<String, toml::Value>,
    key: &'static str,
    field: &'static str,
) -> Result<Option<u32>, ProfileError> {
    optional_flag_integer(table, key, field)?.map_or(Ok(None), |value| {
        u32::try_from(value)
            .map(Some)
            .map_err(|_| invalid(field, "must be an unsigned 32-bit integer"))
    })
}

fn optional_flag_u64(
    table: &toml::map::Map<String, toml::Value>,
    key: &'static str,
    field: &'static str,
) -> Result<Option<u64>, ProfileError> {
    optional_flag_integer(table, key, field)?.map_or(Ok(None), |value| {
        u64::try_from(value)
            .map(Some)
            .map_err(|_| invalid(field, "must be an unsigned integer"))
    })
}

fn optional_compression_algo(
    table: &toml::map::Map<String, toml::Value>,
) -> Result<Option<i32>, ProfileError> {
    const KEY: &str = "data_compress_algo";
    const FIELD: &str = "flags.data_compress_algo";

    match table.get(KEY) {
        None => Ok(None),
        Some(value) => match value {
            toml::Value::Integer(value) => i32::try_from(*value)
                .map(Some)
                .map_err(|_| invalid(FIELD, "must be 1 (none) or 2 (zstd)")),
            // Older hand-written configurations commonly use the human names.
            // The core accepts numeric protobuf values, and we normalize these
            // aliases to those values before rendering a service-owned TOML.
            toml::Value::String(value) if value.eq_ignore_ascii_case("none") => Ok(Some(1)),
            toml::Value::String(value) if value.eq_ignore_ascii_case("zstd") => Ok(Some(2)),
            _ => Err(invalid(FIELD, "must be 1 (none) or 2 (zstd)")),
        },
    }
}

fn required_table<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &'static str,
) -> Result<&'a toml::map::Map<String, toml::Value>, ProfileError> {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| invalid(key, "is required and must be a table"))
}

fn parse_peers(root: &toml::map::Map<String, toml::Value>) -> Result<Vec<String>, ProfileError> {
    let peers = root
        .get("peer")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| invalid("peer", "must be a non-empty array of tables"))?;
    peers
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let table = value
                .as_table()
                .ok_or_else(|| invalid("peer", "entries must be tables"))?;
            ensure_allowed_keys(table, &format!("peer[{index}]"), &["uri"])?;
            required_string(table, "uri")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> NetworkProfile {
        NetworkProfile {
            id: "home".to_owned(),
            name: "Home network".to_owned(),
            instance_name: "home".to_owned(),
            hostname: "laptop".to_owned(),
            network_name: "private-home".to_owned(),
            network_secret: SecretString::new("correct horse battery staple"),
            address_mode: AddressMode::Static {
                cidr: "10.44.0.2/24".to_owned(),
            },
            peers: vec![
                "tcp://seed-a.example.net:11010".to_owned(),
                "udp://seed-b.example.net:11010".to_owned(),
            ],
            flags: EasyTierFlags::default(),
            auto_connect: true,
        }
    }

    #[test]
    fn render_toml_keeps_secret_out_of_process_arguments() {
        let rendered = profile().render_core_toml().unwrap();

        assert!(rendered.contains("private_mode = true"));
        assert!(!rendered.contains("rpc_portal"));
        assert!(rendered.contains("network_secret = \"correct horse battery staple\""));
    }

    #[test]
    fn wireguard_transport_bootstrap_peer_is_accepted_and_rendered() {
        let mut configured = profile();
        configured.peers = vec![
            "tcp://seed.example.net:11010".to_owned(),
            "udp://seed.example.net:11010".to_owned(),
            "wg://seed.example.net:11012".to_owned(),
        ];

        let rendered = configured.render_core_toml().unwrap();

        assert!(rendered.contains("uri = \"wg://seed.example.net:11012\""));
    }

    #[test]
    fn multiple_bootstrap_transports_for_one_seed_are_preserved() {
        let mut configured = profile();
        configured.peers = vec![
            "tcp://seed.example.net:11010".to_owned(),
            "udp://seed.example.net:11010".to_owned(),
            "wg://seed.example.net:11012".to_owned(),
        ];

        let rendered = configured.render_core_toml().unwrap();
        let parsed: toml::Value = toml::from_str(&rendered).unwrap();
        let peer_uris = parsed["peer"]
            .as_array()
            .unwrap()
            .iter()
            .map(|peer| peer["uri"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            peer_uris,
            vec![
                "tcp://seed.example.net:11010",
                "udp://seed.example.net:11010",
                "wg://seed.example.net:11012",
            ]
        );
    }

    #[test]
    fn peer_validation_accepts_an_explicit_wss_default_port_but_requires_a_literal_port() {
        assert!(validate_peer_uri("wss://seed.example.net:443").is_ok());
        assert_eq!(
            validate_peer_uri("wss://seed.example.net"),
            Err(ProfileError::InvalidField {
                field: "peer",
                reason: "must include a host and port".to_owned(),
            })
        );
    }

    #[test]
    fn blank_hostname_uses_the_local_computer_name_before_validation() {
        let mut value = profile();
        value.hostname = " \t ".to_owned();
        value.apply_default_hostname_with(|| "WINDOWS-DESKTOP".to_owned());

        assert_eq!(value.hostname, "WINDOWS-DESKTOP");
        assert!(value.validate().is_ok());
    }

    #[test]
    fn explicit_hostname_is_not_replaced_by_the_local_computer_name() {
        let mut value = profile();
        value.hostname = "work-laptop".to_owned();
        value.apply_default_hostname_with(|| panic!("explicit hostname must not be replaced"));

        assert_eq!(value.hostname, "work-laptop");
    }

    #[test]
    fn whitelisted_import_resolves_a_blank_hostname() {
        let toml = r#"
instance_name = "home"
hostname = ""
ipv4 = "10.44.0.2/24"

[[peer]]
uri = "tcp://seed.example.net:11010"

[network_identity]
network_name = "private-home"
network_secret = "secret"

[flags]
private_mode = true
enable_encryption = true
accept_dns = false
"#;

        let imported = NetworkProfile::from_whitelisted_toml(toml).unwrap();
        assert!(is_usable_hostname(&imported.hostname));
    }

    #[test]
    fn whitelisted_import_accepts_wireguard_transport_bootstrap_peer() {
        let toml = r#"
instance_name = "home"
hostname = "laptop"
ipv4 = "10.44.0.2/24"

[[peer]]
uri = "wg://seed-wg.example.net:11012"

[network_identity]
network_name = "private-home"
network_secret = "secret"

[flags]
private_mode = true
enable_encryption = true
"#;

        let imported = NetworkProfile::from_whitelisted_toml(toml).unwrap();

        assert_eq!(
            imported.peers,
            vec!["wg://seed-wg.example.net:11012".to_owned()]
        );
    }

    #[test]
    fn whitelisted_import_rejects_advanced_core_options() {
        let toml = r#"
instance_name = "home"
hostname = "laptop"
ipv4 = "10.44.0.2/24"
rpc_portal = "0.0.0.0:15888"

[[peer]]
uri = "tcp://seed.example.net:11010"

[network_identity]
network_name = "private-home"
network_secret = "secret"

[flags]
private_mode = true
enable_encryption = true
"#;

        assert_eq!(
            NetworkProfile::from_whitelisted_toml(toml),
            Err(ProfileError::DisallowedTomlKey {
                path: "rpc_portal".to_owned()
            })
        );
    }

    #[test]
    fn whitelisted_import_preserves_explicit_security_flags() {
        let toml = r#"
instance_name = "home"
hostname = "laptop"
ipv4 = "10.44.0.2/24"

[[peer]]
uri = "tcp://seed.example.net:11010"

[network_identity]
network_name = "private-home"
network_secret = "secret"

[flags]
private_mode = false
enable_encryption = false
"#;

        let imported = NetworkProfile::from_whitelisted_toml(toml).unwrap();
        assert!(!imported.flags.private_mode);
        assert!(!imported.flags.enable_encryption);
    }

    #[test]
    fn imports_and_renders_all_v264_flags() {
        let toml = r#"
instance_name = "home"
hostname = "laptop"
ipv4 = "10.44.0.2/24"

[[peer]]
uri = "tcp://seed.example.net:11010"

[network_identity]
network_name = "private-home"
network_secret = "secret"

[flags]
default_protocol = "udp"
dev_name = "VibeTun"
enable_encryption = false
enable_ipv6 = false
mtu = 1300
latency_first = true
enable_exit_node = true
no_tun = true
use_smoltcp = true
relay_network_whitelist = "private-* backup"
disable_p2p = true
relay_all_peer_rpc = true
disable_udp_hole_punching = true
multi_thread = false
data_compress_algo = 2
bind_device = false
enable_kcp_proxy = true
disable_kcp_input = true
disable_relay_kcp = true
proxy_forward_by_system = true
accept_dns = true
private_mode = false
enable_quic_proxy = true
disable_quic_input = true
disable_relay_quic = true
quic_listen_port = 11012
foreign_relay_bps_limit = 123456
multi_thread_count = 4
enable_relay_foreign_network_kcp = true
enable_relay_foreign_network_quic = true
encryption_algorithm = "chacha20"
disable_sym_hole_punching = true
tld_dns_zone = "mesh.internal."
p2p_only = true
disable_tcp_hole_punching = true
lazy_p2p = true
need_p2p = true
instance_recv_bps_limit = 654321
disable_upnp = true
disable_relay_data = true
enable_udp_broadcast_relay = true
"#;

        let imported = NetworkProfile::from_whitelisted_toml(toml).unwrap();
        assert_eq!(imported.flags.default_protocol, "udp");
        assert_eq!(imported.flags.data_compress_algo, 2);
        assert_eq!(imported.flags.foreign_relay_bps_limit, 123456);
        assert_eq!(imported.flags.instance_recv_bps_limit, 654321);
        assert!(imported.flags.enable_udp_broadcast_relay);

        let rendered = imported.render_core_toml().unwrap();
        let parsed: toml::Value = toml::from_str(&rendered).unwrap();
        let rendered_flags = parsed["flags"].as_table().unwrap();
        assert_eq!(EASYTIER_V2_6_4_FLAG_KEYS.len(), EASYTIER_V2_6_4_FLAG_COUNT);
        assert_eq!(rendered_flags.len(), EASYTIER_V2_6_4_FLAG_COUNT);
        for key in EASYTIER_V2_6_4_FLAG_KEYS {
            assert!(
                rendered_flags.contains_key(key),
                "missing rendered flag: {key}"
            );
        }
    }

    #[test]
    fn unlimited_bps_limits_are_omitted_from_toml() {
        let rendered = profile().render_core_toml().unwrap();
        let parsed: toml::Value = toml::from_str(&rendered).unwrap();
        let rendered_flags = parsed["flags"].as_table().unwrap();

        assert!(!rendered_flags.contains_key("foreign_relay_bps_limit"));
        assert!(!rendered_flags.contains_key("instance_recv_bps_limit"));
        assert!(!rendered_flags.contains_key("quic_listen_port"));
        assert_eq!(rendered_flags.len(), EASYTIER_V2_6_4_FLAG_COUNT - 3);
    }

    #[test]
    fn old_profile_state_migrates_to_private_flag_defaults() {
        let mut serialized = serde_json::to_value(profile()).unwrap();
        serialized
            .as_object_mut()
            .expect("profile serializes as an object")
            .remove("flags");

        let migrated: NetworkProfile = serde_json::from_value(serialized).unwrap();
        assert_eq!(migrated.flags, EasyTierFlags::default());
        assert!(migrated.flags.private_mode);
        assert!(migrated.flags.enable_encryption);
        assert!(!migrated.flags.accept_dns);
    }

    #[test]
    fn unknown_v264_flag_is_rejected() {
        let toml = r#"
instance_name = "home"
hostname = "laptop"
ipv4 = "10.44.0.2/24"

[[peer]]
uri = "tcp://seed.example.net:11010"

[network_identity]
network_name = "private-home"
network_secret = "secret"

[flags]
future_core_flag = true
"#;

        assert_eq!(
            NetworkProfile::from_whitelisted_toml(toml),
            Err(ProfileError::DisallowedTomlKey {
                path: "flags.future_core_flag".to_owned(),
            })
        );
    }

    #[cfg(windows)]
    #[test]
    fn bundled_v264_core_accepts_the_complete_rendered_flag_set() {
        use std::{fs, path::PathBuf, process::Command};

        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("service crate lives below the workspace root")
            .to_path_buf();
        let core = workspace_root
            .join("resources")
            .join("easytier")
            .join("windows-x64")
            .join("easytier-core.exe");
        if !core.is_file() {
            // Packaging/Unix test environments do not carry the Windows
            // runtime; the renderer's all-41-field unit test above remains
            // deterministic there.
            return;
        }

        let mut configured = profile();
        configured.peers = vec![
            "tcp://seed.example.net:11010".to_owned(),
            "udp://seed.example.net:11010".to_owned(),
            "wg://seed.example.net:11012".to_owned(),
        ];
        configured.flags.quic_listen_port = 11012;
        configured.flags.foreign_relay_bps_limit = 123_456;
        configured.flags.instance_recv_bps_limit = 654_321;
        let config_path = std::env::temp_dir().join(format!(
            "vibe-easytier-flags-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::write(&config_path, configured.render_core_toml().unwrap()).unwrap();

        let output = {
            use std::os::windows::process::CommandExt;

            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let mut command = Command::new(&core);
            command.args(["-c", config_path.to_str().unwrap(), "--check-config"]);
            command.creation_flags(CREATE_NO_WINDOW);
            command.output()
        };
        let _ = fs::remove_file(&config_path);
        let output = output.expect("bundled EasyTier core should run --check-config");
        assert!(
            output.status.success(),
            "bundled EasyTier core rejected the complete v2.6.4 flag set:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn duplicate_peers_are_rejected() {
        let mut value = profile();
        value.peers.push(value.peers[0].clone());
        assert!(matches!(
            value.validate(),
            Err(ProfileError::InvalidField { field: "peers", .. })
        ));
    }

    #[test]
    fn dhcp_is_rejected_for_new_and_imported_profiles() {
        let mut value = profile();
        value.address_mode = AddressMode::Dhcp;
        assert!(matches!(
            value.validate(),
            Err(ProfileError::InvalidField {
                field: "address mode",
                ..
            })
        ));

        let toml = r#"
instance_name = "home"
hostname = "laptop"
dhcp = true

[[peer]]
uri = "tcp://seed.example.net:11010"

[network_identity]
network_name = "private-home"
network_secret = "secret"

[flags]
private_mode = true
enable_encryption = true
"#;
        assert_eq!(
            NetworkProfile::from_whitelisted_toml(toml),
            Err(ProfileError::DisallowedTomlKey {
                path: "dhcp".to_owned()
            })
        );
    }
}
