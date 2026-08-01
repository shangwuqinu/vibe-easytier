//! Privileged backend primitives for Vibe EasyTier.
//!
//! The desktop application is deliberately not the owner of the EasyTier
//! process.  This crate owns the durable desired state and exposes a small,
//! local-only protocol for a Windows service host.

pub mod bandwidth;
pub mod crypto;
pub mod ipc;
pub mod network;
pub mod profile;
pub mod protocol;
pub mod security;
pub mod service;
pub mod state;
pub mod supervisor;

pub use crypto::{DpapiProtector, StateProtector};
pub use ipc::{Client, IpcClient, IpcError, DEFAULT_PIPE_ENDPOINT};
pub use network::{probe_network_available, NetworkMonitor, NetworkProbeError};
pub use profile::{
    AddressMode, EasyTierFlags, NetworkProfile, ProfileError, SecretString,
    EASYTIER_V2_6_4_FLAG_COUNT, EASYTIER_V2_6_4_FLAG_KEYS,
};
pub use protocol::{
    ConnectedPeer, ConnectionIntent, ProfileUpsert, ProfileView, RpcCommand, RpcRequest,
    RpcResponse, RpcResult, ServiceConnectionState, ServiceLogLine, ServiceStatus,
    PROTOCOL_VERSION,
};
pub use service::{HostMode, ServiceController, ServiceError, ServiceOptions};
pub use state::{PersistedState, ServicePaths, StateStore};
pub use supervisor::{HealthSample, RetryPolicy, Supervisor, SupervisorAction, SupervisorState};
