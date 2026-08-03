//! Windows-service host and durable RPC controller.
//!
//! The Service Control Manager owns this process.  This process, in turn,
//! owns `easytier-core`; the desktop UI only changes durable desired state
//! through the local RPC protocol.

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fmt,
    io::{self, Read},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use thiserror::Error;

use crate::{
    bandwidth::Iperf3BindTarget,
    crypto::{DpapiProtector, StateProtector},
    ipc::{RpcHandler, DEFAULT_PIPE_ENDPOINT},
    profile::{
        AddressMode, EasyTierFlags, NetworkProfile, ProfileError, CORE_RPC_PORTAL,
        CORE_RPC_PORTAL_WHITELIST,
    },
    protocol::{
        ConnectedPeer, ConnectionIntent, ProfileSummary, ProfileUpsert, ProfileView, RpcCommand,
        RpcErrorCode, RpcRequest, RpcResponse, RpcResult, ServiceConnectionState, ServiceLogLine,
        ServiceStatus, PROTOCOL_VERSION,
    },
    security::harden_service_path,
    state::{PersistedState, ServicePaths, StateError, StateStore},
    supervisor::{HealthSample, RetryPolicy, Supervisor, SupervisorAction, SupervisorState},
};

pub const WINDOWS_SERVICE_NAME: &str = crate::ipc::WINDOWS_SERVICE_NAME;
pub const CORE_PATH_ENV: &str = "VIBE_EASYTIER_CORE_PATH";
pub const IPERF3_PATH_ENV: &str = "VIBE_EASYTIER_IPERF3_PATH";
pub const OWNER_SID_ENV: &str = "VIBE_EASYTIER_OWNER_SID";
/// Stable, sanitized marker used when a staged Core config cannot be
/// validated. The desktop maps the small token vocabulary to Chinese text;
/// it must never render raw Core stderr.
pub const CORE_CONFIG_VALIDATION_ERROR_PREFIX: &str =
    "easytier-core configuration validation failed: ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostMode {
    Console,
    Service,
}

#[derive(Clone, Debug)]
pub struct ServiceOptions {
    pub mode: HostMode,
    pub state_root: Option<PathBuf>,
    pub core_executable: Option<PathBuf>,
    pub iperf3_executable: Option<PathBuf>,
    /// The Windows SID allowed to issue mutating local-pipe requests. The
    /// installer passes it with `--owner-sid`; admins always retain access.
    pub owner_sid: Option<String>,
    pub poll_interval: Duration,
}

impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            mode: HostMode::Console,
            state_root: None,
            core_executable: None,
            iperf3_executable: None,
            owner_sid: std::env::var(OWNER_SID_ENV).ok(),
            poll_interval: Duration::from_secs(1),
        }
    }
}

impl ServiceOptions {
    /// Parses arguments after the executable name.  The installer should use
    /// `--service`; `--console` is useful for foreground diagnostics.
    pub fn parse<I, T>(arguments: I) -> Result<Self, ServiceError>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        let mut options = Self::default();
        let mut mode_set = false;
        let mut arguments = arguments
            .into_iter()
            .map(|argument| -> OsString { argument.into() });

        while let Some(argument) = arguments.next() {
            if argument.as_os_str() == OsStr::new("--console") {
                set_mode(&mut options, &mut mode_set, HostMode::Console)?;
            } else if argument.as_os_str() == OsStr::new("--service") {
                set_mode(&mut options, &mut mode_set, HostMode::Service)?;
            } else if argument.as_os_str() == OsStr::new("--state-root") {
                let path = arguments.next().ok_or_else(|| {
                    ServiceError::InvalidArguments("--state-root requires a path".to_owned())
                })?;
                options.state_root = Some(PathBuf::from(path));
            } else if argument.as_os_str() == OsStr::new("--core") {
                let path = arguments.next().ok_or_else(|| {
                    ServiceError::InvalidArguments("--core requires an executable path".to_owned())
                })?;
                options.core_executable = Some(PathBuf::from(path));
            } else if argument.as_os_str() == OsStr::new("--iperf3") {
                let path = arguments.next().ok_or_else(|| {
                    ServiceError::InvalidArguments(
                        "--iperf3 requires an executable path".to_owned(),
                    )
                })?;
                options.iperf3_executable = Some(PathBuf::from(path));
            } else if argument.as_os_str() == OsStr::new("--owner-sid") {
                let owner_sid = arguments.next().ok_or_else(|| {
                    ServiceError::InvalidArguments("--owner-sid requires a Windows SID".to_owned())
                })?;
                let owner_sid = owner_sid.into_string().map_err(|_| {
                    ServiceError::InvalidArguments("--owner-sid must be valid Unicode".to_owned())
                })?;
                if !crate::ipc::is_valid_owner_sid(&owner_sid) {
                    return Err(ServiceError::InvalidArguments(
                        "--owner-sid is not a canonical Windows SID".to_owned(),
                    ));
                }
                options.owner_sid = Some(owner_sid);
            } else if argument.as_os_str() == OsStr::new("--poll-ms") {
                let value = arguments.next().ok_or_else(|| {
                    ServiceError::InvalidArguments(
                        "--poll-ms requires a positive integer".to_owned(),
                    )
                })?;
                let value = value.into_string().map_err(|_| {
                    ServiceError::InvalidArguments("--poll-ms must be valid Unicode".to_owned())
                })?;
                let milliseconds = value.parse::<u64>().map_err(|_| {
                    ServiceError::InvalidArguments(
                        "--poll-ms must be a positive integer".to_owned(),
                    )
                })?;
                if milliseconds == 0 {
                    return Err(ServiceError::InvalidArguments(
                        "--poll-ms must be greater than zero".to_owned(),
                    ));
                }
                options.poll_interval = Duration::from_millis(milliseconds);
            } else if argument.as_os_str() == OsStr::new("--help")
                || argument.as_os_str() == OsStr::new("-h")
            {
                return Err(ServiceError::InvalidArguments(Self::usage().to_owned()));
            } else {
                return Err(ServiceError::InvalidArguments(format!(
                    "unrecognized argument {:?}; {}",
                    argument,
                    Self::usage()
                )));
            }
        }
        if options
            .owner_sid
            .as_deref()
            .is_some_and(|owner_sid| !crate::ipc::is_valid_owner_sid(owner_sid))
        {
            return Err(ServiceError::InvalidArguments(
                "VIBE_EASYTIER_OWNER_SID is not a canonical Windows SID".to_owned(),
            ));
        }
        Ok(options)
    }

    pub const fn usage() -> &'static str {
        "usage: vibe-easytier-service (--service|--console) [--state-root PATH] [--core PATH] [--iperf3 PATH] [--owner-sid SID] [--poll-ms N]"
    }

    pub fn service_paths(&self) -> ServicePaths {
        self.state_root
            .clone()
            .map(ServicePaths::new)
            .unwrap_or_else(ServicePaths::default_for_host)
    }

    fn resolve_core_executable(&self) -> Result<PathBuf, ServiceError> {
        if let Some(path) = &self.core_executable {
            return Ok(path.clone());
        }
        if let Some(path) = std::env::var_os(CORE_PATH_ENV) {
            return Ok(PathBuf::from(path));
        }
        let executable = std::env::current_exe()?;
        let directory = executable.parent().ok_or_else(|| {
            ServiceError::InvalidArguments("service executable has no parent directory".to_owned())
        })?;
        #[cfg(windows)]
        let core_name = "easytier-core.exe";
        #[cfg(not(windows))]
        let core_name = "easytier-core";
        Ok(directory.join(core_name))
    }

    fn resolve_iperf3_executable(&self) -> Result<PathBuf, ServiceError> {
        if let Some(path) = &self.iperf3_executable {
            return Ok(path.clone());
        }
        if let Some(path) = std::env::var_os(IPERF3_PATH_ENV) {
            return Ok(PathBuf::from(path));
        }
        let executable = std::env::current_exe()?;
        let directory = executable.parent().ok_or_else(|| {
            ServiceError::InvalidArguments("service executable has no parent directory".to_owned())
        })?;
        #[cfg(windows)]
        let iperf3_name = "iperf3.exe";
        #[cfg(not(windows))]
        let iperf3_name = "iperf3";
        Ok(directory.join(iperf3_name))
    }
}

fn set_mode(
    options: &mut ServiceOptions,
    mode_set: &mut bool,
    mode: HostMode,
) -> Result<(), ServiceError> {
    if *mode_set && options.mode != mode {
        return Err(ServiceError::InvalidArguments(
            "choose either --service or --console, not both".to_owned(),
        ));
    }
    options.mode = mode;
    *mode_set = true;
    Ok(())
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("invalid service arguments: {0}")]
    InvalidArguments(String),
    #[error("state operation failed: {0}")]
    State(#[from] StateError),
    #[error("invalid profile: {0}")]
    Profile(#[from] ProfileError),
    #[error("Windows service mode is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("core process operation failed: {0}")]
    CoreProcess(#[from] io::Error),
    #[error("core executable does not exist: {0}")]
    MissingCore(PathBuf),
    #[error("{CORE_CONFIG_VALIDATION_ERROR_PREFIX}{0}")]
    CoreConfigRejected(CoreConfigCheckFailure),
    #[error("no active profile is configured")]
    NoActiveProfile,
    #[error("profile {0:?} does not exist")]
    ProfileNotFound(String),
    #[error("service state lock was poisoned")]
    Synchronization,
}

/// Thread-confined service state. Put it behind a mutex if a named-pipe host
/// invokes `handle_rpc` from a different thread than the process supervisor.
pub struct ServiceController<P> {
    store: StateStore<P>,
    config_validator: CoreConfigValidator,
    state: PersistedState,
    retry_policy: RetryPolicy,
    supervisor: Option<Supervisor>,
    core_pid: Option<u32>,
    observed_peer_count: Option<usize>,
    observed_peers: Option<Vec<ConnectedPeer>>,
    observed_route_count: Option<usize>,
    observed_traffic_tx_bytes: Option<u64>,
    observed_traffic_rx_bytes: Option<u64>,
    last_success_unix_ms: Option<u64>,
    last_error: Option<String>,
    reconciliation_needed: bool,
    force_restart: bool,
}

enum CoreConfigValidator {
    Command(PathBuf),
    #[cfg(test)]
    AssumeValid,
    #[cfg(test)]
    Reject(&'static str),
}

impl<P: StateProtector> ServiceController<P> {
    /// Builds a controller which validates every new profile with the bundled
    /// `easytier-core --check-config` before replacing encrypted state.
    pub fn load(
        store: StateStore<P>,
        retry_policy: RetryPolicy,
        core_executable: PathBuf,
    ) -> Result<Self, ServiceError> {
        Self::load_with_validator(
            store,
            retry_policy,
            CoreConfigValidator::Command(core_executable),
        )
    }

    #[cfg(test)]
    fn load_for_tests(
        store: StateStore<P>,
        retry_policy: RetryPolicy,
    ) -> Result<Self, ServiceError> {
        Self::load_with_validator(store, retry_policy, CoreConfigValidator::AssumeValid)
    }

    fn load_with_validator(
        store: StateStore<P>,
        retry_policy: RetryPolicy,
        config_validator: CoreConfigValidator,
    ) -> Result<Self, ServiceError> {
        let state = store.load_or_default()?;
        Ok(Self {
            store,
            config_validator,
            state,
            retry_policy,
            // Do not restore a half-initialized supervisor from encrypted
            // state. `reconcile` needs the current Windows network snapshot
            // before it can decide whether the durable connect intent should
            // launch the core after boot.
            supervisor: None,
            core_pid: None,
            observed_peer_count: None,
            observed_peers: None,
            observed_route_count: None,
            observed_traffic_tx_bytes: None,
            observed_traffic_rx_bytes: None,
            last_success_unix_ms: None,
            last_error: None,
            reconciliation_needed: true,
            force_restart: false,
        })
    }

    pub fn persisted_state(&self) -> &PersistedState {
        &self.state
    }

    pub fn status(&self) -> ServiceStatus {
        let (state, retry_at_unix_ms, consecutive_failures) = self
            .supervisor
            .as_ref()
            .map(|supervisor| {
                (
                    connection_state(supervisor, self.observed_peer_count),
                    supervisor.retry_at_ms(),
                    supervisor.consecutive_failures(),
                )
            })
            .unwrap_or((ServiceConnectionState::Disconnected, None, 0));
        ServiceStatus {
            protocol_version: PROTOCOL_VERSION,
            state,
            active_profile_id: self.state.active_profile_id.clone(),
            auto_connect_profile_id: self
                .state
                .auto_connect_profile()
                .map(|profile| profile.id.clone()),
            core_pid: self.core_pid,
            retry_at_unix_ms,
            consecutive_failures,
            peer_count: self.observed_peer_count.unwrap_or(0),
            peer_count_available: self.observed_peer_count.is_some(),
            route_count: self.observed_route_count.unwrap_or(0),
            traffic_tx_bytes: self.observed_traffic_tx_bytes.unwrap_or(0),
            traffic_rx_bytes: self.observed_traffic_rx_bytes.unwrap_or(0),
            last_success_unix_ms: self.last_success_unix_ms,
            last_error: self.last_error.clone(),
        }
    }

    pub fn profile_summaries(&self) -> Vec<ProfileSummary> {
        self.state
            .profiles
            .values()
            .map(ProfileSummary::from)
            .collect()
    }

    pub fn profile_views(&self) -> Vec<ProfileView> {
        let mut profiles = self
            .state
            .profiles
            .values()
            .map(ProfileView::from)
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.id.cmp(&right.id));
        profiles
    }

    pub fn connected_peers(&self) -> Vec<ConnectedPeer> {
        self.observed_peers.clone().unwrap_or_default()
    }

    pub fn reconcile(&mut self, now_ms: u64, network_available: bool) -> SupervisorAction {
        if !self.reconciliation_needed {
            return SupervisorAction::Noop;
        }
        self.reconciliation_needed = false;
        let force_restart = std::mem::take(&mut self.force_restart);

        let core_running = self.core_pid.is_some();
        let selected = self
            .active_profile()
            .map(|profile| (profile.id.clone(), profile.auto_connect));

        let Some((profile_id, desired)) = selected else {
            self.supervisor = None;
            return if core_running {
                SupervisorAction::StopCore
            } else {
                SupervisorAction::Noop
            };
        };

        let profile_changed = self.supervisor.as_ref().map_or(true, |supervisor| {
            supervisor.profile_id() != profile_id.as_str()
        });
        if profile_changed {
            self.supervisor = Some(Supervisor::new(
                profile_id.clone(),
                desired,
                self.retry_policy.clone(),
            ));
        }

        let supervisor = self
            .supervisor
            .as_mut()
            .expect("selected profile has a supervisor");
        if profile_changed {
            let initial_action = supervisor.initialize(now_ms, network_available);
            if core_running {
                return if desired && network_available {
                    SupervisorAction::RestartCore { profile_id }
                } else {
                    SupervisorAction::StopCore
                };
            }
            return initial_action;
        }

        if core_running && force_restart {
            // `Disconnect` must update the in-memory policy before the child
            // is stopped. Otherwise the next health tick observes no child
            // while the old supervisor still wants one and relaunches it.
            let _ = supervisor.set_desired(desired, now_ms);
            return if desired && network_available {
                SupervisorAction::RestartCore { profile_id }
            } else {
                SupervisorAction::StopCore
            };
        }

        supervisor.set_desired(desired, now_ms)
    }

    pub fn tick(&mut self, now_ms: u64) -> SupervisorAction {
        self.supervisor
            .as_mut()
            .map(|supervisor| supervisor.tick(now_ms))
            .unwrap_or(SupervisorAction::Noop)
    }

    pub fn on_network_changed(&mut self, network_available: bool, now_ms: u64) -> SupervisorAction {
        self.supervisor
            .as_mut()
            .map(|supervisor| supervisor.on_network_changed(network_available, now_ms))
            .unwrap_or(SupervisorAction::Noop)
    }

    pub fn on_system_resume(&mut self, now_ms: u64, network_available: bool) -> SupervisorAction {
        self.supervisor
            .as_mut()
            .map(|supervisor| supervisor.on_system_resume(now_ms, network_available))
            .unwrap_or(SupervisorAction::Noop)
    }

    pub fn on_core_started(&mut self, pid: u32, now_ms: u64) -> SupervisorAction {
        self.core_pid = Some(pid);
        self.observed_peer_count = None;
        self.observed_peers = None;
        self.observed_route_count = None;
        self.observed_traffic_tx_bytes = None;
        self.observed_traffic_rx_bytes = None;
        self.last_error = None;
        self.supervisor
            .as_mut()
            .map(|supervisor| supervisor.on_core_started(now_ms))
            .unwrap_or(SupervisorAction::StopCore)
    }

    pub fn on_core_exited(&mut self, now_ms: u64, detail: impl Into<String>) {
        self.core_pid = None;
        self.observed_peer_count = None;
        self.observed_peers = None;
        self.observed_route_count = None;
        self.observed_traffic_tx_bytes = None;
        self.observed_traffic_rx_bytes = None;
        self.last_error = Some(detail.into());
        if let Some(supervisor) = &mut self.supervisor {
            supervisor.on_core_exited(now_ms);
        }
    }

    pub fn on_core_stopped(&mut self) {
        self.core_pid = None;
        self.observed_peer_count = None;
        self.observed_peers = None;
        self.observed_route_count = None;
        self.observed_traffic_tx_bytes = None;
        self.observed_traffic_rx_bytes = None;
    }

    pub fn on_health_sample(&mut self, sample: HealthSample, now_ms: u64) -> SupervisorAction {
        if sample.control_plane_healthy {
            self.last_success_unix_ms = Some(now_ms);
        }
        self.observed_peer_count = sample.connected_peer_count;
        self.observed_peers = sample.connected_peers.clone();
        if let Some(route_count) = sample.route_count {
            self.observed_route_count = Some(route_count);
        }
        if let Some(traffic_tx_bytes) = sample.traffic_tx_bytes {
            self.observed_traffic_tx_bytes = Some(traffic_tx_bytes);
        }
        if let Some(traffic_rx_bytes) = sample.traffic_rx_bytes {
            self.observed_traffic_rx_bytes = Some(traffic_rx_bytes);
        }
        self.supervisor
            .as_mut()
            .map(|supervisor| supervisor.on_health_sample(sample, now_ms))
            .unwrap_or(SupervisorAction::Noop)
    }

    pub fn record_error(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
    }

    fn active_profile(&self) -> Option<&NetworkProfile> {
        self.state
            .active_profile_id
            .as_deref()
            .and_then(|profile_id| self.state.profiles.get(profile_id))
    }

    fn iperf3_bind_target(&self) -> Option<Iperf3BindTarget> {
        if self.core_pid.is_none() {
            return None;
        }
        let AddressMode::Static { cidr } = &self.active_profile()?.address_mode else {
            return None;
        };
        Iperf3BindTarget::from_cidr(cidr)
    }

    fn prepare_core_launch(&self, expected_profile_id: &str) -> Result<CoreLaunch, ServiceError> {
        let profile = self.active_profile().ok_or(ServiceError::NoActiveProfile)?;
        if profile.id.as_str() != expected_profile_id {
            return Err(ServiceError::NoActiveProfile);
        }
        let config_path = self.store.write_runtime_profile(profile)?;
        let log_dir = self.store.paths().logs_dir();
        std::fs::create_dir_all(&log_dir)?;
        harden_service_path(&log_dir)?;
        Ok(CoreLaunch {
            config_path,
            log_dir,
        })
    }

    fn upsert_profile(&mut self, input: ProfileUpsert) -> Result<ProfileView, ServiceError> {
        let ProfileUpsert {
            mut profile,
            make_active,
        } = input;
        profile.apply_default_hostname();
        profile.validate()?;
        self.validate_profile_with_core(&profile)?;
        let profile_id = profile.id.clone();
        let was_active = self.state.active_profile_id.as_deref() == Some(profile_id.as_str());
        let mut next = self.state.clone();

        if make_active || profile.auto_connect {
            for profile in next.profiles.values_mut() {
                profile.auto_connect = false;
            }
            next.active_profile_id = Some(profile_id.clone());
        } else if next.active_profile_id.is_none() {
            next.active_profile_id = Some(profile_id.clone());
        }
        next.profiles.insert(profile_id.clone(), profile);

        let is_active = next.active_profile_id.as_deref() == Some(profile_id.as_str());
        self.commit(next, was_active || is_active)?;
        self.state
            .profiles
            .get(&profile_id)
            .map(ProfileView::from)
            .ok_or(ServiceError::ProfileNotFound(profile_id))
    }

    /// Updates an existing profile's EasyTier flags without returning or
    /// replacing its network secret. `upsert_profile` validates the rendered
    /// staged TOML with core before committing encrypted state, so an invalid
    /// settings change leaves the active profile and its live connection
    /// untouched.
    fn update_profile_flags(
        &mut self,
        profile_id: String,
        flags: EasyTierFlags,
    ) -> Result<ProfileView, ServiceError> {
        let was_active = self.state.active_profile_id.as_deref() == Some(profile_id.as_str());
        let mut profile = self
            .state
            .profiles
            .get(&profile_id)
            .cloned()
            .ok_or_else(|| ServiceError::ProfileNotFound(profile_id.clone()))?;
        profile.flags = flags;

        self.upsert_profile(ProfileUpsert {
            profile,
            make_active: was_active,
        })
    }

    fn import_profile(
        &mut self,
        toml: String,
        make_active: bool,
    ) -> Result<ProfileView, ServiceError> {
        let profile = NetworkProfile::from_whitelisted_toml(&toml)?;
        self.upsert_profile(ProfileUpsert {
            profile,
            make_active,
        })
    }

    fn export_profile_toml(&self, profile_id: &str) -> Result<String, ServiceError> {
        self.state
            .profiles
            .get(profile_id)
            .ok_or_else(|| ServiceError::ProfileNotFound(profile_id.to_owned()))?
            .render_core_toml()
            .map_err(ServiceError::Profile)
    }

    fn delete_profile(&mut self, profile_id: &str) -> Result<(), ServiceError> {
        if !self.state.profiles.contains_key(profile_id) {
            return Err(ServiceError::ProfileNotFound(profile_id.to_owned()));
        }
        let was_active = self.state.active_profile_id.as_deref() == Some(profile_id);
        let mut next = self.state.clone();
        next.profiles.remove(profile_id);
        if was_active {
            next.active_profile_id = next.profiles.keys().next().cloned();
        }
        self.commit(next, was_active)
    }

    fn select_active_profile(&mut self, profile_id: &str) -> Result<(), ServiceError> {
        if !self.state.profiles.contains_key(profile_id) {
            return Err(ServiceError::ProfileNotFound(profile_id.to_owned()));
        }
        let was_active = self.state.active_profile_id.as_deref() == Some(profile_id);
        let mut next = self.state.clone();
        for profile in next.profiles.values_mut() {
            profile.auto_connect = false;
        }
        next.active_profile_id = Some(profile_id.to_owned());
        self.commit(next, !was_active)
    }

    fn apply_intent(&mut self, intent: ConnectionIntent) -> Result<(), ServiceError> {
        let mut next = self.state.clone();
        let restart = match intent {
            ConnectionIntent::Connect { profile_id } => {
                if !next.profiles.contains_key(&profile_id) {
                    return Err(ServiceError::ProfileNotFound(profile_id));
                }
                for profile in next.profiles.values_mut() {
                    profile.auto_connect = false;
                }
                let profile = next
                    .profiles
                    .get_mut(&profile_id)
                    .expect("profile existence was checked");
                profile.auto_connect = true;
                next.active_profile_id = Some(profile_id);
                true
            }
            ConnectionIntent::Disconnect { profile_id } => {
                let profile_id = profile_id
                    .or_else(|| next.active_profile_id.clone())
                    .ok_or(ServiceError::NoActiveProfile)?;
                let profile = next
                    .profiles
                    .get_mut(&profile_id)
                    .ok_or_else(|| ServiceError::ProfileNotFound(profile_id.clone()))?;
                profile.auto_connect = false;
                true
            }
            ConnectionIntent::SetAutoConnect {
                profile_id,
                enabled,
            } => {
                if !next.profiles.contains_key(&profile_id) {
                    return Err(ServiceError::ProfileNotFound(profile_id));
                }
                if enabled {
                    for profile in next.profiles.values_mut() {
                        profile.auto_connect = false;
                    }
                    next.active_profile_id = Some(profile_id.clone());
                }
                let profile = next
                    .profiles
                    .get_mut(&profile_id)
                    .expect("profile existence was checked");
                profile.auto_connect = enabled;
                next.active_profile_id.as_deref() == Some(profile_id.as_str())
            }
        };
        self.commit(next, restart)
    }

    fn commit(&mut self, next: PersistedState, force_restart: bool) -> Result<(), ServiceError> {
        next.validate()?;
        self.store.save(&next)?;
        self.state = next;
        self.reconciliation_needed = true;
        self.force_restart |= force_restart;
        Ok(())
    }

    fn validate_profile_with_core(&self, profile: &NetworkProfile) -> Result<(), ServiceError> {
        match &self.config_validator {
            CoreConfigValidator::Command(executable) => {
                if !executable.is_file() {
                    return Err(ServiceError::MissingCore(executable.clone()));
                }

                let staged_config = self.store.write_staged_runtime_profile(profile)?;
                let validation = check_core_config(executable, &staged_config);
                let cleanup = std::fs::remove_file(&staged_config);
                match (validation, cleanup) {
                    (Ok(()), Ok(())) => Ok(()),
                    (Err(error), _) => Err(error),
                    (Ok(()), Err(error)) => Err(ServiceError::CoreProcess(error)),
                }
            }
            #[cfg(test)]
            CoreConfigValidator::AssumeValid => Ok(()),
            #[cfg(test)]
            CoreConfigValidator::Reject(reason) => Err(ServiceError::CoreConfigRejected(
                CoreConfigCheckFailure::exited(None, reason.as_bytes()),
            )),
        }
    }

    fn tail_logs(&self, requested_limit: usize) -> Result<Vec<ServiceLogLine>, ServiceError> {
        const MAX_LOG_RECORDS: usize = 500;
        let limit = requested_limit.clamp(1, MAX_LOG_RECORDS);
        let directory = self.store.paths().logs_dir();
        if !directory.exists() {
            return Ok(Vec::new());
        }

        let mut files = std::fs::read_dir(directory)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let metadata = entry.metadata().ok()?;
                metadata.is_file().then(|| {
                    (
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        entry.path(),
                    )
                })
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| right.0.cmp(&left.0));

        let secrets = self
            .state
            .profiles
            .values()
            .map(|profile| profile.network_secret.expose())
            .collect::<Vec<_>>();
        let mut records = Vec::with_capacity(limit);
        for (_, path) in files {
            let source = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("easytier-core.log")
                .to_owned();
            let content = std::fs::read_to_string(path)?;
            for record in parse_core_log_records(&content).into_iter().rev() {
                let Some(record) = sanitize_core_log_record(&record, &secrets) else {
                    continue;
                };
                records.push(ServiceLogLine {
                    source: source.clone(),
                    line: record,
                });
                if records.len() == limit {
                    return Ok(records);
                }
            }
        }
        Ok(records)
    }

    fn clear_logs(&self) -> Result<(), ServiceError> {
        let directory = self.store.paths().logs_dir();
        if !directory.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
}

fn parse_core_log_records(content: &str) -> Vec<String> {
    let has_structured_records = content.lines().any(is_core_log_record_start);
    if !has_structured_records {
        return content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_owned)
            .collect();
    }

    let mut records = Vec::new();
    let mut current = None::<String>;
    for line in content.lines() {
        if is_core_log_record_start(line) {
            if let Some(record) = current.take() {
                records.push(record.trim_end().to_owned());
            }
            current = Some(line.to_owned());
        } else if let Some(record) = current.as_mut() {
            record.push('\n');
            record.push_str(line);
        } else if !line.trim().is_empty() {
            // Preserve an incomplete leading fragment from a rotated log file.
            records.push(line.to_owned());
        }
    }
    if let Some(record) = current {
        records.push(record.trim_end().to_owned());
    }
    records
}

fn is_core_log_record_start(line: &str) -> bool {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    let line = line.strip_prefix('[').unwrap_or(line);
    let bytes = line.as_bytes();
    if bytes.len() < 19 {
        return false;
    }

    let digit = |index: usize| bytes[index].is_ascii_digit();
    (0..4).all(digit)
        && bytes[4] == b'-'
        && (5..7).all(digit)
        && bytes[7] == b'-'
        && (8..10).all(digit)
        && matches!(bytes[10], b'T' | b' ')
        && (11..13).all(digit)
        && bytes[13] == b':'
        && (14..16).all(digit)
        && bytes[16] == b':'
        && (17..19).all(digit)
}

fn sanitize_core_log_record(record: &str, secrets: &[&str]) -> Option<String> {
    let mut sanitized = record
        .lines()
        .filter(|line| !line.contains("--network-secret") && !line.contains("network_secret"))
        .collect::<Vec<_>>()
        .join("\n");
    for secret in secrets {
        if !secret.is_empty() {
            sanitized = sanitized.replace(secret, "[redacted]");
        }
    }
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

impl<P: StateProtector> RpcHandler for ServiceController<P> {
    fn handle_rpc(&mut self, request: RpcRequest) -> RpcResponse {
        if request.protocol_version != PROTOCOL_VERSION {
            return RpcResponse::error(
                request.request_id,
                RpcErrorCode::UnsupportedVersion,
                format!(
                    "protocol version {} is unsupported",
                    request.protocol_version
                ),
            );
        }

        let result = match request.command {
            RpcCommand::Ping => Ok(RpcResult::Pong),
            RpcCommand::GetStatus => Ok(RpcResult::Status(self.status())),
            RpcCommand::ListProfiles => Ok(RpcResult::Profiles(self.profile_views())),
            RpcCommand::ListPeers => Ok(RpcResult::Peers(self.connected_peers())),
            RpcCommand::UpsertProfile(input) => {
                self.upsert_profile(input).map(RpcResult::ProfileSaved)
            }
            RpcCommand::UpdateProfileFlags { profile_id, flags } => self
                .update_profile_flags(profile_id, flags)
                .map(RpcResult::ProfileSaved),
            RpcCommand::ImportProfile { toml, make_active } => self
                .import_profile(toml, make_active)
                .map(RpcResult::ProfileSaved),
            RpcCommand::ExportProfile { profile_id } => self
                .export_profile_toml(&profile_id)
                .map(|toml| RpcResult::ProfileToml { profile_id, toml }),
            RpcCommand::DeleteProfile { profile_id } => self
                .delete_profile(&profile_id)
                .map(|()| RpcResult::ProfileDeleted { profile_id }),
            RpcCommand::SetActiveProfile { profile_id } => self
                .select_active_profile(&profile_id)
                .map(|()| RpcResult::ActiveProfileSelected(self.status())),
            RpcCommand::SetConnectionIntent { intent } => self
                .apply_intent(intent)
                .map(|()| RpcResult::IntentApplied(self.status())),
            RpcCommand::TailLogs { limit } => self.tail_logs(limit).map(RpcResult::Logs),
            RpcCommand::ClearLogs => self.clear_logs().map(|()| RpcResult::LogsCleared),
        };

        match result {
            Ok(result) => RpcResponse::ok(request.request_id, result),
            Err(error) => RpcResponse::error(
                request.request_id,
                rpc_error_code(&error),
                error.to_string(),
            ),
        }
    }
}

fn connection_state(supervisor: &Supervisor, peer_count: Option<usize>) -> ServiceConnectionState {
    match supervisor.state() {
        SupervisorState::Stopped => ServiceConnectionState::Disconnected,
        SupervisorState::WaitingForNetwork | SupervisorState::Starting => {
            ServiceConnectionState::Connecting
        }
        // A running core can be healthy before it has joined a remote peer.
        // Do not report a private network as connected until a successful CLI
        // health sample has observed at least one non-local peer.
        SupervisorState::Running if peer_count.is_some_and(|count| count > 0) => {
            ServiceConnectionState::Connected
        }
        SupervisorState::Running => ServiceConnectionState::Connecting,
        SupervisorState::BackingOff => ServiceConnectionState::Recovering,
        SupervisorState::Degraded => ServiceConnectionState::Failed,
    }
}

fn rpc_error_code(error: &ServiceError) -> RpcErrorCode {
    match error {
        ServiceError::ProfileNotFound(_) | ServiceError::NoActiveProfile => RpcErrorCode::NotFound,
        ServiceError::Profile(_)
        | ServiceError::State(StateError::Profile(_))
        | ServiceError::CoreConfigRejected(_) => RpcErrorCode::InvalidProfile,
        ServiceError::State(StateError::InvalidState(_)) => RpcErrorCode::Conflict,
        ServiceError::InvalidArguments(_) => RpcErrorCode::InvalidRequest,
        ServiceError::UnsupportedPlatform | ServiceError::MissingCore(_) => {
            RpcErrorCode::Unavailable
        }
        ServiceError::State(_) | ServiceError::CoreProcess(_) | ServiceError::Synchronization => {
            RpcErrorCode::Internal
        }
    }
}

struct CoreLaunch {
    config_path: PathBuf,
    log_dir: PathBuf,
}

const CORE_CONFIG_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CORE_CONFIG_STDERR_BYTES: usize = 16 * 1024;

/// A deliberately small, non-sensitive result of `easytier-core
/// --check-config`. Its display form only contains a fixed token and, when
/// available, an exit code. In particular, it never retains Core stderr,
/// which may contain the staged config's path or network secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreConfigCheckFailure(CoreConfigCheckOutcome);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoreConfigCheckOutcome {
    TimedOut,
    Exited {
        code: Option<i32>,
        reason: CoreConfigFailureReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoreConfigFailureReason {
    NetworkIdentity,
    VirtualAddress,
    BootstrapPeer,
    CoreOption,
    TomlFormat,
    FileAccess,
    Unknown,
}

impl CoreConfigCheckFailure {
    const fn timed_out() -> Self {
        Self(CoreConfigCheckOutcome::TimedOut)
    }

    fn exited(code: Option<i32>, stderr: &[u8]) -> Self {
        Self(CoreConfigCheckOutcome::Exited {
            code,
            reason: classify_core_config_stderr(stderr),
        })
    }
}

impl fmt::Display for CoreConfigCheckFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            CoreConfigCheckOutcome::TimedOut => formatter.write_str("validation timed out"),
            CoreConfigCheckOutcome::Exited {
                code: Some(code),
                reason,
            } => write!(formatter, "exit code {code}; reason={}", reason.token()),
            CoreConfigCheckOutcome::Exited { code: None, reason } => write!(
                formatter,
                "terminated by the operating system; reason={}",
                reason.token()
            ),
        }
    }
}

impl CoreConfigFailureReason {
    const fn token(self) -> &'static str {
        match self {
            Self::NetworkIdentity => "network_identity",
            Self::VirtualAddress => "virtual_address",
            Self::BootstrapPeer => "bootstrap_peer",
            Self::CoreOption => "core_option",
            Self::TomlFormat => "toml_format",
            Self::FileAccess => "file_access",
            Self::Unknown => "unknown",
        }
    }
}

/// Classifies stderr without ever returning any of its text. Error output from
/// Core can include the generated TOML location and literal values, including
/// the private network secret, so only a fixed allowlist of broad categories
/// may cross the service IPC boundary.
fn classify_core_config_stderr(stderr: &[u8]) -> CoreConfigFailureReason {
    let normalized = String::from_utf8_lossy(stderr).to_ascii_lowercase();

    if contains_any(
        &normalized,
        &[
            "network_secret",
            "network secret",
            "network_name",
            "network name",
        ],
    ) {
        CoreConfigFailureReason::NetworkIdentity
    } else if contains_any(&normalized, &["ipv4", "ipv6", "cidr", "prefix length"]) {
        CoreConfigFailureReason::VirtualAddress
    } else if contains_any(&normalized, &["bootstrap", "peer", "peer uri", "uri"]) {
        CoreConfigFailureReason::BootstrapPeer
    } else if contains_any(
        &normalized,
        &[
            "[flags]",
            "flags.",
            "mtu",
            "data_compress",
            "compression",
            "encryption_algorithm",
            "default_protocol",
            "quic",
            "kcp",
            "p2p",
            "upnp",
            "relay",
            "no_tun",
            "smoltcp",
            "accept_dns",
            "broadcast",
        ],
    ) {
        CoreConfigFailureReason::CoreOption
    } else if contains_any(
        &normalized,
        &[
            "permission denied",
            "access is denied",
            "os error 5",
            "no such file or directory",
            "the system cannot find the file",
        ],
    ) {
        CoreConfigFailureReason::FileAccess
    } else if contains_any(
        &normalized,
        &[
            "toml",
            "parse",
            "deserialize",
            "invalid type",
            "expected ",
            "missing field",
        ],
    ) {
        CoreConfigFailureReason::TomlFormat
    } else {
        CoreConfigFailureReason::Unknown
    }
}

fn contains_any(value: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| value.contains(term))
}

/// Reads just enough diagnostic data for classification, while continuing to
/// drain the pipe so a noisy Core process cannot block before it exits.
fn capture_core_stderr(mut stderr: impl Read) -> io::Result<Vec<u8>> {
    let mut captured = Vec::with_capacity(MAX_CORE_CONFIG_STDERR_BYTES);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stderr.read(&mut buffer)?;
        if read == 0 {
            return Ok(captured);
        }
        let remaining = MAX_CORE_CONFIG_STDERR_BYTES.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
}

fn join_core_stderr(
    reader: thread::JoinHandle<io::Result<Vec<u8>>>,
) -> Result<Vec<u8>, ServiceError> {
    let stderr = reader.join().map_err(|_| {
        ServiceError::CoreProcess(io::Error::other(
            "Core configuration stderr reader unexpectedly stopped",
        ))
    })??;
    Ok(stderr)
}

fn check_core_config(
    executable: &std::path::Path,
    config_path: &std::path::Path,
) -> Result<(), ServiceError> {
    let mut command = Command::new(executable);
    command
        .args(core_config_check_arguments(config_path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    let stderr = child.stderr.take().ok_or_else(|| {
        ServiceError::CoreProcess(io::Error::other(
            "Core configuration stderr was not captured",
        ))
    })?;
    let stderr_reader = thread::spawn(move || capture_core_stderr(stderr));
    let deadline = Instant::now() + CORE_CONFIG_CHECK_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(ServiceError::CoreConfigRejected(
                    CoreConfigCheckFailure::timed_out(),
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(ServiceError::CoreProcess(error));
            }
        }
    };
    let stderr = join_core_stderr(stderr_reader)?;
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(ServiceError::CoreConfigRejected(
            CoreConfigCheckFailure::exited(status.code(), &stderr),
        ))
    }
}

fn core_config_check_arguments(config_path: &std::path::Path) -> Vec<OsString> {
    vec![
        OsString::from("-c"),
        config_path.as_os_str().to_owned(),
        OsString::from("--check-config"),
    ]
}

fn core_command_arguments(
    config_path: &std::path::Path,
    log_dir: &std::path::Path,
) -> Vec<OsString> {
    vec![
        OsString::from("-c"),
        config_path.as_os_str().to_owned(),
        // `rpc_portal` is a core CLI option rather than a merged TOML field.
        // Pinning it here prevents the core's default wildcard binding.
        OsString::from("--rpc-portal"),
        OsString::from(CORE_RPC_PORTAL),
        OsString::from("--rpc-portal-whitelist"),
        OsString::from(CORE_RPC_PORTAL_WHITELIST),
        OsString::from("--file-log-dir"),
        log_dir.as_os_str().to_owned(),
        OsString::from("--file-log-level"),
        OsString::from("info"),
        OsString::from("--file-log-size"),
        OsString::from("5"),
        OsString::from("--file-log-count"),
        OsString::from("5"),
    ]
}

/// Process boundary used by the service loop. Keeping this boundary small lets
/// the controller/recovery integration tests exercise the same action path as
/// the Windows host without spawning a real EasyTier instance.
trait CoreRuntime<P: StateProtector> {
    /// Starts (or replaces) the child for `profile_id` and returns its PID.
    fn start(
        &mut self,
        controller: &ServiceController<P>,
        options: &ServiceOptions,
        profile_id: &str,
    ) -> Result<u32, ServiceError>;

    /// Returns a diagnostic when the managed child has exited since the last
    /// poll. A successful `None` means it is still running or absent.
    fn poll_exit(&mut self) -> Result<Option<String>, ServiceError>;

    fn stop(&mut self) -> Result<(), ServiceError>;

    fn health_sample(&mut self) -> Result<HealthSample, ServiceError>;
}

struct CoreRunner {
    executable: Option<PathBuf>,
    child: Option<Child>,
    #[cfg(windows)]
    job: Option<CoreJob>,
}

impl CoreRunner {
    fn new(executable: Option<PathBuf>) -> Self {
        Self {
            executable,
            child: None,
            #[cfg(windows)]
            job: None,
        }
    }

    fn start<P: StateProtector>(
        &mut self,
        controller: &ServiceController<P>,
        options: &ServiceOptions,
        profile_id: &str,
    ) -> Result<u32, ServiceError> {
        self.stop()?;
        let launch = controller.prepare_core_launch(profile_id)?;
        let executable = self
            .executable
            .clone()
            .unwrap_or(options.resolve_core_executable()?);
        if !executable.is_file() {
            return Err(ServiceError::MissingCore(executable));
        }

        let mut command = Command::new(&executable);
        command
            .args(core_command_arguments(&launch.config_path, &launch.log_dir))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn()?;
        #[cfg(windows)]
        {
            if let Err(error) = self.assign_to_job(&child) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        let pid = child.id();
        self.child = Some(child);
        self.executable = Some(executable);
        Ok(pid)
    }

    fn poll_child_exit(&mut self) -> Result<Option<ExitStatus>, ServiceError> {
        let Some(child) = &mut self.child else {
            return Ok(None);
        };
        let status = child.try_wait()?;
        if status.is_some() {
            self.child = None;
        }
        Ok(status)
    }

    fn health_sample(&mut self) -> Result<HealthSample, ServiceError> {
        let running = self.poll_child_exit()?.is_none() && self.child.is_some();
        let (peer_output, route_output, stats_output) = match (running, self.cli_executable()) {
            (true, Some(executable)) => {
                // The three read-only CLI calls are independent. Run them in
                // parallel so one telemetry timeout does not triple the time
                // the service controller is occupied by a health probe.
                let route_executable = executable.clone();
                let route_task =
                    thread::spawn(move || run_route_list_cli(&route_executable).ok().flatten());
                let stats_executable = executable.clone();
                let stats_task =
                    thread::spawn(move || run_stats_cli(&stats_executable).ok().flatten());
                let peer_output = run_peer_list_cli(&executable).ok().flatten();
                (
                    peer_output,
                    route_task.join().ok().flatten(),
                    stats_task.join().ok().flatten(),
                )
            }
            _ => (None, None, None),
        };
        // A TCP connect alone can succeed while a hung core never processes a
        // management request. A bounded sidecar CLI response is therefore the
        // authoritative control-plane proof; parsing peer rows is telemetry.
        let control_plane_healthy = running && core_rpc_reachable() && peer_output.is_some();
        let peer_snapshot = peer_output.as_deref().and_then(peers_from_cli_json);
        let route_count = route_output.as_deref().and_then(route_count_from_cli_json);
        let traffic = stats_output.as_deref().and_then(traffic_from_cli_json);
        // `peer list` includes the local node with cost=Local. The service
        // status and recovery policy must count only remote peers so that a
        // lonely core remains eligible for the conservative no-peer restart.
        let connected_peer_count = peer_snapshot
            .as_ref()
            .map(|peers| peers.iter().filter(|peer| !is_local_peer(peer)).count());
        let connected_peers = peer_snapshot.map(|peers| {
            peers
                .into_iter()
                .filter(|peer| !is_local_peer(peer))
                .collect()
        });
        Ok(HealthSample {
            core_process_running: running,
            control_plane_healthy,
            private_network_reachable: None,
            connected_peer_count,
            connected_peers,
            route_count,
            traffic_tx_bytes: traffic.map(|traffic| traffic.tx_bytes),
            traffic_rx_bytes: traffic.map(|traffic| traffic.rx_bytes),
        })
    }

    fn cli_executable(&self) -> Option<PathBuf> {
        let core = self.executable.as_ref()?;
        let directory = core.parent()?;
        #[cfg(windows)]
        let cli_name = "easytier-cli.exe";
        #[cfg(not(windows))]
        let cli_name = "easytier-cli";
        let cli = directory.join(cli_name);
        cli.is_file().then_some(cli)
    }

    fn stop(&mut self) -> Result<(), ServiceError> {
        #[cfg(windows)]
        let terminated_by_job = self.job.as_ref().is_some_and(|job| job.terminate().is_ok());
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if child.try_wait()?.is_none() {
            #[cfg(not(windows))]
            child.kill()?;
            #[cfg(windows)]
            if !terminated_by_job {
                child.kill()?;
            }
            let _ = child.wait()?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn assign_to_job(&mut self, child: &Child) -> Result<(), ServiceError> {
        use std::os::windows::io::AsRawHandle;

        if self.job.is_none() {
            self.job = Some(CoreJob::new()?);
        }
        self.job
            .as_ref()
            .expect("job was initialized")
            .assign(child.as_raw_handle() as isize)
            .map_err(ServiceError::CoreProcess)
    }
}

impl<P: StateProtector> CoreRuntime<P> for CoreRunner {
    fn start(
        &mut self,
        controller: &ServiceController<P>,
        options: &ServiceOptions,
        profile_id: &str,
    ) -> Result<u32, ServiceError> {
        CoreRunner::start(self, controller, options, profile_id)
    }

    fn poll_exit(&mut self) -> Result<Option<String>, ServiceError> {
        CoreRunner::poll_child_exit(self)
            .map(|status| status.map(|status| format!("easytier-core exited with status {status}")))
    }

    fn stop(&mut self) -> Result<(), ServiceError> {
        CoreRunner::stop(self)
    }

    fn health_sample(&mut self) -> Result<HealthSample, ServiceError> {
        CoreRunner::health_sample(self)
    }
}

/// Owns a Windows Job Object with kill-on-close semantics. If SCM terminates
/// the service process unexpectedly, Windows closes this handle and tears down
/// the complete EasyTier process tree rather than leaving a stale core behind.
#[cfg(windows)]
struct CoreJob {
    handle: isize,
}

#[cfg(windows)]
impl CoreJob {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if handle == 0 {
            return Err(io::Error::last_os_error());
        }
        let limits = JobObjectExtendedLimitInformation {
            basic_limit_information: JobObjectBasicLimitInformation {
                limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..JobObjectBasicLimitInformation::default()
            },
            ..JobObjectExtendedLimitInformation::default()
        };
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &limits as *const JobObjectExtendedLimitInformation as *const std::ffi::c_void,
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            )
        };
        if configured == 0 {
            unsafe {
                CloseHandle(handle);
            }
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    fn assign(&self, process_handle: isize) -> io::Result<()> {
        let assigned = unsafe { AssignProcessToJobObject(self.handle, process_handle) };
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn terminate(&self) -> io::Result<()> {
        let terminated = unsafe { TerminateJobObject(self.handle, 1) };
        if terminated == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for CoreJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[cfg(windows)]
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
#[cfg(windows)]
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(security_attributes: *mut std::ffi::c_void, name: *const u16) -> isize;
    fn SetInformationJobObject(
        job: isize,
        information_class: u32,
        information: *const std::ffi::c_void,
        information_length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
    fn TerminateJobObject(job: isize, exit_code: u32) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

fn core_rpc_reachable() -> bool {
    let address = CORE_RPC_PORTAL
        .parse::<SocketAddr>()
        .expect("the service-owned EasyTier RPC portal must be a socket address");
    TcpStream::connect_timeout(&address, Duration::from_millis(750)).is_ok()
}

const CLI_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CLI_OUTPUT_BYTES: u64 = 1024 * 1024;
const MAX_PROTOCOLS_PER_PEER: usize = 12;
const MAX_PROTOCOL_TOKEN_BYTES: usize = 48;
const MAX_PROTOCOL_PARTS_PER_VALUE: usize = MAX_PROTOCOLS_PER_PEER * 4;

/// Runs the sidecar CLI with a strict timeout. The stdout reader drains in a
/// separate thread so a large response cannot deadlock the child on a pipe
/// buffer before the timeout is evaluated.
fn run_peer_list_cli(executable: &std::path::Path) -> io::Result<Option<String>> {
    run_cli_json(executable, &["peer", "list"])
}

fn run_route_list_cli(executable: &std::path::Path) -> io::Result<Option<String>> {
    run_cli_json(executable, &["route", "list"])
}

fn run_stats_cli(executable: &std::path::Path) -> io::Result<Option<String>> {
    run_cli_json(executable, &["stats", "show"])
}

fn run_cli_json(executable: &std::path::Path, subcommand: &[&str]) -> io::Result<Option<String>> {
    let mut command = Command::new(executable);
    command
        .args(["-p", CORE_RPC_PORTAL, "-o", "json"])
        .args(subcommand)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .expect("stdout was explicitly configured as piped");
    let reader = thread::spawn(move || -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_CLI_OUTPUT_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    });
    let deadline = Instant::now() + CLI_PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Ok(None);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(error);
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| io::Error::other("EasyTier CLI stdout reader panicked"))??;
    if !status.success() || bytes.len() as u64 > MAX_CLI_OUTPUT_BYTES {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

#[cfg(test)]
fn peer_count_from_cli_json(output: &str) -> Option<usize> {
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    peer_count_from_cli_value(&value)
}

#[cfg(test)]
fn peer_count_from_cli_value(value: &serde_json::Value) -> Option<usize> {
    match value {
        serde_json::Value::Array(entries) => Some(entries.len()),
        serde_json::Value::Object(object) => [
            "peers",
            "peer_infos",
            "peer_info",
            "peer_list",
            "items",
            "data",
            "result",
        ]
        .iter()
        .find_map(|key| object.get(*key).and_then(peer_count_from_cli_value)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoreTraffic {
    tx_bytes: u64,
    rx_bytes: u64,
}

fn route_count_from_cli_json(output: &str) -> Option<usize> {
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    route_entries_from_cli_value(&value).map(Vec::len)
}

fn route_entries_from_cli_value(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Array(entries) => Some(entries),
        serde_json::Value::Object(object) => ["routes", "route_list", "items", "data", "result"]
            .iter()
            .find_map(|key| object.get(*key).and_then(route_entries_from_cli_value)),
        _ => None,
    }
}

fn traffic_from_cli_json(output: &str) -> Option<CoreTraffic> {
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    let entries = stats_entries_from_cli_value(&value)?;
    let mut self_tx = None;
    let mut self_rx = None;
    let mut data_tx = None;
    let mut data_rx = None;

    for entry in entries {
        let Some(object) = entry.as_object() else {
            continue;
        };
        let Some(name) = cli_string(object, &["name"]) else {
            continue;
        };
        let Some(value) = cli_number(object, &["value"]) else {
            continue;
        };
        match name.as_str() {
            "traffic_bytes_self_tx" => saturating_option_add(&mut self_tx, value),
            "traffic_bytes_self_rx" => saturating_option_add(&mut self_rx, value),
            "traffic_bytes_tx" => saturating_option_add(&mut data_tx, value),
            "traffic_bytes_rx" => saturating_option_add(&mut data_rx, value),
            _ => {}
        }
    }

    Some(CoreTraffic {
        tx_bytes: self_tx.or(data_tx)?,
        rx_bytes: self_rx.or(data_rx)?,
    })
}

fn stats_entries_from_cli_value(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Array(entries) => Some(entries),
        serde_json::Value::Object(object) => ["stats", "metrics", "items", "data", "result"]
            .iter()
            .find_map(|key| object.get(*key).and_then(stats_entries_from_cli_value)),
        _ => None,
    }
}

fn saturating_option_add(total: &mut Option<u64>, value: u64) {
    *total = Some(total.unwrap_or_default().saturating_add(value));
}

fn peers_from_cli_json(output: &str) -> Option<Vec<ConnectedPeer>> {
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    peer_entries_from_cli_value(&value).map(|entries| {
        merge_connected_peer_rows(
            entries
                .iter()
                .filter_map(connected_peer_from_cli_value)
                .collect(),
        )
    })
}

fn peer_entries_from_cli_value(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Array(entries) => Some(entries),
        serde_json::Value::Object(object) => [
            "peers",
            "peer_infos",
            "peer_info",
            "peer_list",
            "items",
            "data",
            "result",
        ]
        .iter()
        .find_map(|key| object.get(*key).and_then(peer_entries_from_cli_value)),
        _ => None,
    }
}

fn connected_peer_from_cli_value(value: &serde_json::Value) -> Option<ConnectedPeer> {
    let object = value.as_object()?;
    let id = cli_string(object, &["id", "peer_id"])?;
    if id.is_empty() {
        return None;
    }
    let cidr = cli_string(object, &["cidr"]);
    let ipv4 = cli_string(object, &["ipv4", "virtual_ip"])
        .or_else(|| {
            cidr.as_deref()
                .and_then(|value| value.split('/').next().map(str::to_owned))
        })
        .unwrap_or_else(|| "--".to_owned());
    let hostname =
        cli_string(object, &["hostname", "host_name", "name"]).unwrap_or_else(|| id.clone());
    let protocols = cli_protocols(object);
    Some(ConnectedPeer {
        id,
        hostname,
        ipv4,
        cidr,
        cost: cli_string(object, &["cost"]),
        latency_ms: cli_number(object, &["lat_ms", "latency_ms"])
            .and_then(|value| u32::try_from(value).ok()),
        rx_bytes: cli_bytes(object, &["rx_bytes", "received", "received_bytes"]),
        tx_bytes: cli_bytes(object, &["tx_bytes", "sent", "sent_bytes"]),
        tunnel_protocol: protocol_csv(&protocols),
        protocols,
        nat_type: cli_string(object, &["nat_type"]),
        version: cli_string(object, &["version"]),
    })
}

/// EasyTier v2.6.4 serializes the active connection transports as one
/// comma-separated `tunnel_proto` string. Accept an array too, so a future
/// CLI can expose the same information structurally without changing the
/// service API. Only compact, known transport labels are allowed across the
/// local IPC boundary.
fn cli_protocols(object: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let mut protocols = Vec::new();
    for key in [
        "tunnel_proto",
        "tunnel_protocol",
        "tunnel_protos",
        "tunnel_protocols",
    ] {
        let Some(value) = object.get(key) else {
            continue;
        };
        append_cli_protocols(value, &mut protocols);
        if protocols.len() == MAX_PROTOCOLS_PER_PEER {
            break;
        }
    }
    protocols
}

fn append_cli_protocols(value: &serde_json::Value, protocols: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) => {
            for raw_protocol in value.split(',').take(MAX_PROTOCOL_PARTS_PER_VALUE) {
                let Some(protocol) = sanitize_tunnel_protocol(raw_protocol) else {
                    continue;
                };
                if !protocols.contains(&protocol) {
                    protocols.push(protocol);
                    if protocols.len() == MAX_PROTOCOLS_PER_PEER {
                        break;
                    }
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values.iter().take(MAX_PROTOCOL_PARTS_PER_VALUE) {
                if protocols.len() == MAX_PROTOCOLS_PER_PEER {
                    break;
                }
                append_cli_protocols(value, protocols);
            }
        }
        _ => {}
    }
}

fn sanitize_tunnel_protocol(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value == "-"
        || value.len() > MAX_PROTOCOL_TOKEN_BYTES
        || !value.is_ascii()
    {
        return None;
    }

    let value = value.to_ascii_lowercase();
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
    }) {
        return None;
    }

    let (prefix, transport) = value.rsplit_once('-').unwrap_or(("", value.as_str()));
    let transport = transport.strip_suffix('6').unwrap_or(transport);
    let accepted = match transport.split_once('_') {
        Some(("faketcp", driver)) => matches!(driver, "pnet" | "windivert" | "bpf"),
        Some(_) => false,
        None => matches!(
            transport,
            "tcp" | "udp" | "wg" | "ws" | "wss" | "quic" | "faketcp" | "ring" | "unix"
        ),
    };
    if !accepted {
        return None;
    }

    // These are the connector prefixes emitted by the EasyTier v2.6.4
    // runtime. Keeping the set closed prevents a peer's arbitrary metadata
    // from being displayed as a transport label.
    if !prefix.is_empty() && !matches!(prefix, "txt" | "srv" | "http" | "https" | "dns") {
        return None;
    }

    Some(value)
}

fn protocol_csv(protocols: &[String]) -> Option<String> {
    (!protocols.is_empty()).then(|| protocols.join(","))
}

/// The CLI normally returns one row per remote peer. Merge by stable peer ID
/// nevertheless: a multi-connection Core or a future CLI shape must not make
/// the desktop show duplicate nodes or inflate the peer health count.
fn merge_connected_peer_rows(rows: Vec<ConnectedPeer>) -> Vec<ConnectedPeer> {
    let mut peers = Vec::with_capacity(rows.len());
    let mut indexes = HashMap::with_capacity(rows.len());
    for peer in rows {
        if let Some(index) = indexes.get(&peer.id).copied() {
            merge_connected_peer_protocols(&mut peers[index], peer.protocols);
            continue;
        }

        let index = peers.len();
        indexes.insert(peer.id.clone(), index);
        peers.push(peer);
    }
    peers
}

fn merge_connected_peer_protocols(peer: &mut ConnectedPeer, incoming: Vec<String>) {
    for protocol in incoming {
        if peer.protocols.len() == MAX_PROTOCOLS_PER_PEER {
            break;
        }
        if !peer.protocols.contains(&protocol) {
            peer.protocols.push(protocol);
        }
    }
    peer.tunnel_protocol = protocol_csv(&peer.protocols);
}

fn cli_string(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| match value {
            serde_json::Value::String(value) if value != "-" => Some(value.to_owned()),
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn cli_number(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| match value {
            serde_json::Value::Number(value) => value.as_u64(),
            serde_json::Value::String(value) if value != "-" => value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite() && *number >= 0.0)
                .map(|number| number.round().min(u64::MAX as f64) as u64),
            _ => None,
        })
    })
}

fn cli_bytes(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| match value {
            serde_json::Value::Number(value) => value.as_u64(),
            serde_json::Value::String(value) if value != "-" => parse_human_bytes(value),
            _ => None,
        })
    })
}

fn parse_human_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    let unit_start = value
        .char_indices()
        .find_map(|(index, character)| {
            (!character.is_ascii_digit() && character != '.' && character != ',').then_some(index)
        })
        .unwrap_or(value.len());
    let number = value[..unit_start].replace(',', "").parse::<f64>().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    let multiplier = match value[unit_start..].trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1_f64,
        "kb" | "kib" => 1_024_f64,
        "mb" | "mib" => 1_024_f64.powi(2),
        "gb" | "gib" => 1_024_f64.powi(3),
        "tb" | "tib" => 1_024_f64.powi(4),
        _ => return None,
    };
    Some((number * multiplier).round().min(u64::MAX as f64) as u64)
}

fn is_local_peer(peer: &ConnectedPeer) -> bool {
    peer.cost
        .as_deref()
        .is_some_and(|cost| cost.eq_ignore_ascii_case("local"))
}

/// Runs the real parent/child supervision loop until `stop` becomes true.
/// The current host intentionally treats a failed bootstrap as a core failure;
/// EasyTier then benefits from the same capped retry policy whether startup
/// happens before or after a user logs in.
pub fn run_until_stopped(options: ServiceOptions, stop: &AtomicBool) -> Result<(), ServiceError> {
    #[cfg(not(windows))]
    {
        let _ = (options, stop);
        return Err(ServiceError::UnsupportedPlatform);
    }

    #[cfg(windows)]
    {
        let core_executable = options.resolve_core_executable()?;
        if !core_executable.is_file() {
            return Err(ServiceError::MissingCore(core_executable));
        }
        let iperf3_executable = options.resolve_iperf3_executable()?;
        let store = StateStore::new(options.service_paths(), DpapiProtector);
        let controller = Arc::new(Mutex::new(ServiceController::load(
            store,
            RetryPolicy::default(),
            core_executable.clone(),
        )?));
        let ipc_controller = Arc::clone(&controller);
        let ipc_stop = Arc::new(AtomicBool::new(false));
        let ipc_stop_for_thread = Arc::clone(&ipc_stop);
        let ipc_owner_sid = options.owner_sid.clone();
        let _ipc_thread = thread::Builder::new()
            .name("vibe-easytier-ipc".to_owned())
            .spawn(move || {
                if let Err(error) = crate::ipc::serve_windows_pipe_until(
                    DEFAULT_PIPE_ENDPOINT,
                    ipc_owner_sid.as_deref(),
                    &ipc_stop_for_thread,
                    &ipc_controller,
                ) {
                    if let Ok(mut controller) = ipc_controller.lock() {
                        controller.record_error(format!("named-pipe server stopped: {error}"));
                    }
                }
            })
            .map_err(ServiceError::CoreProcess)?;
        let bandwidth_target = Arc::new(Mutex::new(None));
        let bandwidth_target_for_thread = Arc::clone(&bandwidth_target);
        let bandwidth_stop = Arc::new(AtomicBool::new(false));
        let bandwidth_stop_for_thread = Arc::clone(&bandwidth_stop);
        let bandwidth_thread = thread::Builder::new()
            .name("vibe-easytier-bandwidth".to_owned())
            .spawn(move || {
                let _ = crate::bandwidth::serve_iperf3_until(
                    &iperf3_executable,
                    &bandwidth_stop_for_thread,
                    &bandwidth_target_for_thread,
                );
            })
            .ok();
        let mut runner = CoreRunner::new(Some(core_executable));
        let mut network_monitor = crate::network::NetworkMonitor::new();
        let mut previous_loop_at = Instant::now();

        while !stop.load(Ordering::Acquire) {
            let loop_started_at = Instant::now();
            let resumed = is_resume_gap(
                loop_started_at.saturating_duration_since(previous_loop_at),
                options.poll_interval,
            );
            previous_loop_at = loop_started_at;
            let now_ms = unix_time_ms();
            if let Ok(Some(network_available)) = network_monitor.refresh() {
                let mut controller_guard = controller
                    .lock()
                    .map_err(|_| ServiceError::Synchronization)?;
                let action = controller_guard.on_network_changed(network_available, now_ms);
                apply_supervisor_action(
                    action,
                    &mut controller_guard,
                    &mut runner,
                    &options,
                    now_ms,
                );
            }

            let network_available = network_monitor.available();
            if resumed {
                let mut controller_guard = controller
                    .lock()
                    .map_err(|_| ServiceError::Synchronization)?;
                let action = controller_guard.on_system_resume(now_ms, network_available);
                apply_supervisor_action(
                    action,
                    &mut controller_guard,
                    &mut runner,
                    &options,
                    now_ms,
                );
            }
            {
                let mut controller_guard = controller
                    .lock()
                    .map_err(|_| ServiceError::Synchronization)?;
                let action = controller_guard.reconcile(now_ms, network_available);
                apply_supervisor_action(
                    action,
                    &mut controller_guard,
                    &mut runner,
                    &options,
                    now_ms,
                );
            }

            {
                let mut controller_guard = controller
                    .lock()
                    .map_err(|_| ServiceError::Synchronization)?;
                observe_core_exit(&mut runner, &mut controller_guard, now_ms);
            }

            {
                let mut controller_guard = controller
                    .lock()
                    .map_err(|_| ServiceError::Synchronization)?;
                let action = controller_guard.tick(now_ms);
                apply_supervisor_action(
                    action,
                    &mut controller_guard,
                    &mut runner,
                    &options,
                    now_ms,
                );
            }
            {
                let desired_target = controller
                    .lock()
                    .map_err(|_| ServiceError::Synchronization)?
                    .iperf3_bind_target();
                *bandwidth_target
                    .lock()
                    .map_err(|_| ServiceError::Synchronization)? = desired_target;
            }
            thread::sleep(options.poll_interval);
        }

        runner.stop()?;
        controller
            .lock()
            .map_err(|_| ServiceError::Synchronization)?
            .on_core_stopped();
        *bandwidth_target
            .lock()
            .map_err(|_| ServiceError::Synchronization)? = None;
        bandwidth_stop.store(true, Ordering::Release);
        if let Some(bandwidth_thread) = bandwidth_thread {
            let _ = bandwidth_thread.join();
        }
        ipc_stop.store(true, Ordering::Release);
        Ok(())
    }
}

fn is_resume_gap(elapsed: Duration, poll_interval: Duration) -> bool {
    elapsed > poll_interval.saturating_mul(3)
}

fn observe_core_exit<P: StateProtector, R: CoreRuntime<P>>(
    runner: &mut R,
    controller: &mut ServiceController<P>,
    now_ms: u64,
) {
    match runner.poll_exit() {
        Ok(Some(detail)) => controller.on_core_exited(now_ms, detail),
        Ok(None) => {}
        Err(error) => controller.record_error(error.to_string()),
    }
}

fn apply_supervisor_action<P: StateProtector, R: CoreRuntime<P>>(
    mut action: SupervisorAction,
    controller: &mut ServiceController<P>,
    runner: &mut R,
    options: &ServiceOptions,
    now_ms: u64,
) {
    // A start can immediately become a stop if the desired state changed while
    // the process was being created. Bound the follow-up sequence defensively.
    for _ in 0..3 {
        action = match action {
            SupervisorAction::Noop => break,
            SupervisorAction::StopCore => {
                if let Err(error) = runner.stop() {
                    controller.record_error(error.to_string());
                }
                controller.on_core_stopped();
                SupervisorAction::Noop
            }
            SupervisorAction::StartCore { profile_id }
            | SupervisorAction::RestartCore { profile_id } => {
                // `CoreRunner::start` first kills an existing child. Clear the
                // visible PID before spawn so a failed restart cannot report a
                // stale process as connected.
                controller.on_core_stopped();
                match runner.start(controller, options, &profile_id) {
                    Ok(pid) => controller.on_core_started(pid, now_ms),
                    Err(error) => {
                        controller.record_error(error.to_string());
                        if let Some(supervisor) = &mut controller.supervisor {
                            supervisor.on_core_start_failed(now_ms);
                        }
                        SupervisorAction::Noop
                    }
                }
            }
            SupervisorAction::ProbeHealth => match runner.health_sample() {
                Ok(sample) if sample.core_process_running => {
                    controller.on_health_sample(sample, now_ms)
                }
                Ok(_) => {
                    controller.on_core_exited(now_ms, "easytier-core exited during health probe");
                    SupervisorAction::Noop
                }
                Err(error) => {
                    controller.record_error(error.to_string());
                    SupervisorAction::Noop
                }
            },
        };
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

pub fn run_console(options: ServiceOptions) -> Result<(), ServiceError> {
    #[cfg(not(windows))]
    {
        let _ = options;
        return Err(ServiceError::UnsupportedPlatform);
    }

    #[cfg(windows)]
    {
        let stop = Arc::new(AtomicBool::new(false));
        let signal_stop = Arc::clone(&stop);
        ctrlc::set_handler(move || signal_stop.store(true, Ordering::Release)).map_err(
            |error| {
                ServiceError::InvalidArguments(format!("could not install Ctrl-C handler: {error}"))
            },
        )?;
        run_until_stopped(options, &stop)
    }
}

#[cfg(windows)]
pub mod windows {
    use std::{
        ffi::OsString,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, OnceLock,
        },
        time::Duration,
    };

    use windows_service::{
        define_windows_service,
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState,
            ServiceStatus as WindowsServiceStatus, ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
    };

    use super::{run_until_stopped, ServiceOptions, WINDOWS_SERVICE_NAME};

    static SERVICE_OPTIONS: OnceLock<ServiceOptions> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    pub fn dispatch_service() -> windows_service::Result<()> {
        dispatch_service_with_options(ServiceOptions {
            mode: super::HostMode::Service,
            ..ServiceOptions::default()
        })
    }

    pub fn dispatch_service_with_options(options: ServiceOptions) -> windows_service::Result<()> {
        let _ = SERVICE_OPTIONS.set(options);
        service_dispatcher::start(WINDOWS_SERVICE_NAME, ffi_service_main)
    }

    fn service_main(_arguments: Vec<OsString>) {
        let _ = run_service();
    }

    fn run_service() -> windows_service::Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_handler = Arc::clone(&stop);
        let status_handle =
            service_control_handler::register(WINDOWS_SERVICE_NAME, move |event| match event {
                ServiceControl::Stop => {
                    stop_for_handler.store(true, Ordering::Release);
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            })?;

        status_handle.set_service_status(status(
            ServiceState::StartPending,
            1,
            Duration::from_secs(10),
        ))?;
        status_handle.set_service_status(status(ServiceState::Running, 0, Duration::default()))?;

        let options = SERVICE_OPTIONS
            .get()
            .cloned()
            .unwrap_or_else(|| ServiceOptions {
                mode: super::HostMode::Service,
                ..ServiceOptions::default()
            });
        let result = run_until_stopped(options, &stop);
        let exit_code = if result.is_ok() { 0 } else { 1 };
        let _ = status_handle.set_service_status(WindowsServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(exit_code),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });
        Ok(())
    }

    fn status(state: ServiceState, checkpoint: u32, wait_hint: Duration) -> WindowsServiceStatus {
        WindowsServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(0),
            checkpoint,
            wait_hint,
            process_id: None,
        }
    }
}

#[cfg(not(windows))]
pub mod windows {
    use super::ServiceError;

    pub fn dispatch_service() -> Result<(), ServiceError> {
        Err(ServiceError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, fs, io::Cursor};

    use crate::{
        crypto::TestProtector,
        ipc::RpcHandler,
        profile::{AddressMode, SecretString},
        protocol::{ConnectionIntent, RpcCommand, RpcRequest},
    };

    use super::*;

    fn paths(name: &str) -> ServicePaths {
        ServicePaths::new(std::env::temp_dir().join(format!(
            "vibe-easytier-service-controller-{name}-{}",
            std::process::id()
        )))
    }

    fn profile(id: &str) -> NetworkProfile {
        NetworkProfile {
            id: id.to_owned(),
            name: id.to_owned(),
            instance_name: id.to_owned(),
            hostname: "laptop".to_owned(),
            network_name: "private-network".to_owned(),
            network_secret: SecretString::new("test-secret"),
            address_mode: AddressMode::Static {
                cidr: "10.44.0.2/24".to_owned(),
            },
            peers: vec!["tcp://seed.example.net:11010".to_owned()],
            flags: crate::profile::EasyTierFlags::default(),
            auto_connect: false,
        }
    }

    fn controller(name: &str) -> ServiceController<TestProtector> {
        controller_with_policy(name, RetryPolicy::default())
    }

    fn controller_with_policy(
        name: &str,
        retry_policy: RetryPolicy,
    ) -> ServiceController<TestProtector> {
        let paths = paths(name);
        let _ = fs::remove_dir_all(paths.root());
        ServiceController::load_for_tests(StateStore::new(paths, TestProtector), retry_policy)
            .unwrap()
    }

    fn fake_core_policy() -> RetryPolicy {
        RetryPolicy {
            initial_backoff: Duration::from_millis(10),
            maximum_backoff: Duration::from_millis(40),
            health_probe_interval: Duration::from_millis(1),
            stable_connection_window: Duration::from_millis(20),
            max_consecutive_failures_before_degraded: 3,
            max_consecutive_health_failures: 2,
            no_peer_restart_after: Duration::from_millis(10),
            no_peer_restart_min_interval: Duration::from_millis(15),
        }
    }

    fn auto_connect_controller(name: &str) -> ServiceController<TestProtector> {
        let mut controller = controller_with_policy(name, fake_core_policy());
        let response = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        assert!(response.error.is_none());
        let response = controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::SetConnectionIntent {
                intent: ConnectionIntent::Connect {
                    profile_id: "home".to_owned(),
                },
            },
        ));
        assert!(response.error.is_none());
        controller
    }

    #[derive(Default)]
    struct FakeCore {
        running: bool,
        exit_on_next_poll: bool,
        next_pid: u32,
        starts: usize,
        replacements: usize,
        stops: usize,
        health_samples: VecDeque<HealthSample>,
    }

    impl FakeCore {
        fn exit_on_next_poll(&mut self) {
            self.exit_on_next_poll = true;
        }

        fn queue_health(&mut self, sample: HealthSample) {
            self.health_samples.push_back(sample);
        }
    }

    impl CoreRuntime<TestProtector> for FakeCore {
        fn start(
            &mut self,
            _controller: &ServiceController<TestProtector>,
            _options: &ServiceOptions,
            _profile_id: &str,
        ) -> Result<u32, ServiceError> {
            if self.running {
                // Production CoreRunner::start replaces an existing child.
                self.replacements += 1;
            }
            self.running = true;
            self.starts += 1;
            self.next_pid = self.next_pid.saturating_add(1);
            Ok(self.next_pid)
        }

        fn poll_exit(&mut self) -> Result<Option<String>, ServiceError> {
            if self.running && self.exit_on_next_poll {
                self.running = false;
                self.exit_on_next_poll = false;
                return Ok(Some("fake core exited immediately".to_owned()));
            }
            Ok(None)
        }

        fn stop(&mut self) -> Result<(), ServiceError> {
            if self.running {
                self.running = false;
                self.stops += 1;
            }
            Ok(())
        }

        fn health_sample(&mut self) -> Result<HealthSample, ServiceError> {
            assert!(
                self.running,
                "a stopped fake core must not be health-probed"
            );
            Ok(self
                .health_samples
                .pop_front()
                .expect("test must queue each requested fake-core health sample"))
        }
    }

    fn apply_fake_action(
        action: SupervisorAction,
        controller: &mut ServiceController<TestProtector>,
        core: &mut FakeCore,
        now_ms: u64,
    ) {
        apply_supervisor_action(action, controller, core, &ServiceOptions::default(), now_ms);
    }

    fn healthy_sample(peer_count: usize) -> HealthSample {
        let connected_peers = (0..peer_count)
            .map(|index| ConnectedPeer {
                id: format!("peer-{index}"),
                hostname: format!("node-{index}"),
                ipv4: format!("10.44.0.{}", index + 3),
                cidr: Some(format!("10.44.0.{}/24", index + 3)),
                cost: Some("10".to_owned()),
                latency_ms: Some(5),
                rx_bytes: None,
                tx_bytes: None,
                protocols: Vec::new(),
                tunnel_protocol: None,
                nat_type: None,
                version: None,
            })
            .collect();
        HealthSample {
            core_process_running: true,
            control_plane_healthy: true,
            private_network_reachable: None,
            connected_peer_count: Some(peer_count),
            connected_peers: Some(connected_peers),
            route_count: Some(peer_count.saturating_add(1)),
            traffic_tx_bytes: Some(1_024),
            traffic_rx_bytes: Some(2_048),
        }
    }

    fn hung_rpc_sample() -> HealthSample {
        HealthSample {
            core_process_running: true,
            control_plane_healthy: false,
            private_network_reachable: None,
            connected_peer_count: None,
            connected_peers: None,
            route_count: None,
            traffic_tx_bytes: None,
            traffic_rx_bytes: None,
        }
    }

    #[test]
    fn fake_core_immediate_exit_retries_without_losing_auto_connect_intent() {
        let mut controller = auto_connect_controller("fake-immediate-exit");
        let mut core = FakeCore::default();

        apply_fake_action(controller.reconcile(0, true), &mut controller, &mut core, 0);
        assert_eq!(core.starts, 1);
        assert_eq!(controller.status().core_pid, Some(1));

        core.exit_on_next_poll();
        observe_core_exit(&mut core, &mut controller, 1);

        assert!(!core.running);
        assert!(controller.persisted_state().profiles["home"].auto_connect);
        assert_eq!(controller.status().core_pid, None);
        assert_eq!(
            controller.status().state,
            ServiceConnectionState::Recovering
        );
        assert_eq!(
            controller.status().last_error.as_deref(),
            Some("fake core exited immediately")
        );

        let retry_at = controller
            .supervisor
            .as_ref()
            .and_then(Supervisor::retry_at_ms)
            .expect("an immediately exited core must be retried");
        apply_fake_action(
            controller.tick(retry_at),
            &mut controller,
            &mut core,
            retry_at,
        );

        assert_eq!(core.starts, 2);
        assert!(core.running);
        assert_eq!(controller.status().core_pid, Some(2));
    }

    #[test]
    fn fake_core_hung_rpc_is_restarted_after_the_health_threshold() {
        let mut controller = auto_connect_controller("fake-hung-rpc");
        let mut core = FakeCore::default();
        core.queue_health(hung_rpc_sample());
        core.queue_health(hung_rpc_sample());

        apply_fake_action(controller.reconcile(0, true), &mut controller, &mut core, 0);
        apply_fake_action(controller.tick(1), &mut controller, &mut core, 1);
        assert_eq!(core.starts, 1);
        assert_eq!(core.replacements, 0);

        apply_fake_action(controller.tick(2), &mut controller, &mut core, 2);

        assert_eq!(core.starts, 2);
        assert_eq!(core.replacements, 1);
        assert!(core.running);
        assert_eq!(controller.status().core_pid, Some(2));
        assert_eq!(
            controller.status().state,
            ServiceConnectionState::Connecting
        );
    }

    #[test]
    fn fake_core_no_peers_restarts_conservatively_then_reports_recovered_peer() {
        let mut controller = auto_connect_controller("fake-no-peer-recovery");
        let mut core = FakeCore::default();
        core.queue_health(healthy_sample(0));
        core.queue_health(healthy_sample(0));
        core.queue_health(healthy_sample(1));

        apply_fake_action(controller.reconcile(0, true), &mut controller, &mut core, 0);
        apply_fake_action(controller.tick(1), &mut controller, &mut core, 1);
        assert_eq!(core.starts, 1, "a lone healthy core is not restarted early");
        assert_eq!(
            controller.status().state,
            ServiceConnectionState::Connecting
        );
        assert_eq!(controller.status().peer_count, 0);
        assert!(controller.status().peer_count_available);

        apply_fake_action(controller.tick(11), &mut controller, &mut core, 11);
        assert_eq!(
            core.starts, 2,
            "the no-peer timeout replaces the child once"
        );
        assert_eq!(core.replacements, 1);

        apply_fake_action(controller.tick(12), &mut controller, &mut core, 12);
        let status = controller.status();
        assert_eq!(status.state, ServiceConnectionState::Connected);
        assert_eq!(status.peer_count, 1);
        assert!(status.peer_count_available);
        assert_eq!(
            core.starts, 2,
            "a recovered peer does not trigger another restart"
        );
    }

    #[test]
    fn fake_core_waits_offline_and_restarts_when_network_returns() {
        let mut controller = auto_connect_controller("fake-network-recovery");
        let mut core = FakeCore::default();

        apply_fake_action(controller.reconcile(0, true), &mut controller, &mut core, 0);
        assert_eq!(core.starts, 1);

        apply_fake_action(
            controller.on_network_changed(false, 1),
            &mut controller,
            &mut core,
            1,
        );
        assert_eq!(core.stops, 1);
        assert!(!core.running);
        assert_eq!(controller.status().core_pid, None);
        assert!(controller.persisted_state().profiles["home"].auto_connect);
        assert_eq!(controller.tick(100), SupervisorAction::Noop);

        apply_fake_action(
            controller.on_network_changed(true, 101),
            &mut controller,
            &mut core,
            101,
        );
        assert_eq!(core.starts, 2);
        assert!(core.running);
        assert_eq!(controller.status().core_pid, Some(2));
    }

    #[test]
    fn connect_intent_is_durable_and_starts_after_service_restart() {
        let paths = paths("auto-connect");
        let _ = fs::remove_dir_all(paths.root());
        let mut controller = ServiceController::load_for_tests(
            StateStore::new(paths.clone(), TestProtector),
            RetryPolicy::default(),
        )
        .unwrap();
        let response = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        assert!(response.error.is_none());

        let response = controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::SetConnectionIntent {
                intent: ConnectionIntent::Connect {
                    profile_id: "home".to_owned(),
                },
            },
        ));
        assert!(response.error.is_none());
        assert!(matches!(
            controller.reconcile(1, true),
            SupervisorAction::StartCore { profile_id } if profile_id == "home"
        ));
        assert!(controller.persisted_state().profiles["home"].auto_connect);

        drop(controller);
        let mut restarted = ServiceController::load_for_tests(
            StateStore::new(paths.clone(), TestProtector),
            RetryPolicy::default(),
        )
        .unwrap();
        assert!(matches!(
            restarted.reconcile(2, true),
            SupervisorAction::StartCore { profile_id } if profile_id == "home"
        ));
        let _ = fs::remove_dir_all(paths.root());
    }

    #[test]
    fn disconnect_stops_and_persists_the_non_reconnecting_intent() {
        let mut controller = controller("disconnect");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::SetConnectionIntent {
                intent: ConnectionIntent::Connect {
                    profile_id: "home".to_owned(),
                },
            },
        ));
        controller.reconcile(1, true);
        controller.on_core_started(123, 2);

        let response = controller.handle_rpc(RpcRequest::new(
            3,
            RpcCommand::SetConnectionIntent {
                intent: ConnectionIntent::Disconnect { profile_id: None },
            },
        ));
        assert!(response.error.is_none());
        assert_eq!(controller.reconcile(3, true), SupervisorAction::StopCore);
        controller.on_core_stopped();
        assert_eq!(controller.tick(100), SupervisorAction::Noop);
        assert_eq!(
            controller.status().state,
            ServiceConnectionState::Disconnected
        );
        assert!(!controller.persisted_state().profiles["home"].auto_connect);
    }

    #[test]
    fn failed_restart_does_not_leave_the_previous_pid_in_status() {
        let mut controller = controller("failed-restart");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::SetConnectionIntent {
                intent: ConnectionIntent::Connect {
                    profile_id: "home".to_owned(),
                },
            },
        ));
        controller.reconcile(1, true);
        controller.on_core_started(456, 2);

        // The host clears the old PID before it attempts a replacement child.
        controller.on_core_stopped();
        controller
            .supervisor
            .as_mut()
            .expect("active profile has a supervisor")
            .on_core_start_failed(3);

        assert_eq!(controller.status().core_pid, None);
        assert_eq!(
            controller.status().state,
            ServiceConnectionState::Recovering
        );
    }

    #[test]
    fn log_tail_redacts_network_material() {
        let mut controller = controller("log-tail");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        let log_dir = controller.store.paths().logs_dir();
        fs::create_dir_all(&log_dir).unwrap();
        fs::write(
            log_dir.join("easytier-core.log"),
            "ordinary line\npeer joined with test-secret\n--network-secret test-secret\n",
        )
        .unwrap();

        let logs = controller.tail_logs(10).unwrap();
        assert!(logs.iter().all(|line| !line.line.contains("test-secret")));
        assert!(logs.iter().any(|line| line.line == "ordinary line"));
    }

    #[test]
    fn log_tail_groups_multiline_core_events_into_single_records() {
        let controller = controller("log-tail-multiline");
        let log_dir = controller.store.paths().logs_dir();
        fs::create_dir_all(&log_dir).unwrap();
        fs::write(
            log_dir.join("easytier-core.log"),
            concat!(
                "2026-08-01T15:29:22.000Z  INFO easytier::launcher: Core started\n",
                "2026-08-01T15:29:23.000Z  INFO easytier::connector: lookup failed: Error {\n",
                "    context: \"hickory dns lookup_ip failed\",\n",
                "    source: ResolveError {\n",
                "        kind: Proto(\n",
                "            ProtoError {\n",
                "                response_code: NXDomain,\n",
                "            },\n",
                "        ),\n",
                "    },\n",
                "}\n",
                "2026-08-01T15:29:24.000Z  WARN easytier::connector: retry scheduled\n",
            ),
        )
        .unwrap();

        let logs = controller.tail_logs(10).unwrap();
        assert_eq!(logs.len(), 3);
        assert!(logs[0].line.contains("retry scheduled"));
        assert!(logs[1].line.contains("lookup failed: Error {"));
        assert!(logs[1].line.contains("response_code: NXDomain"));
        assert_eq!(logs[1].line.lines().count(), 10);
        assert!(logs[2].line.contains("Core started"));

        let latest = controller.tail_logs(1).unwrap();
        assert_eq!(latest.len(), 1);
        assert!(latest[0].line.contains("retry scheduled"));
    }

    #[test]
    fn clear_logs_removes_service_owned_log_history() {
        let controller = controller("clear-logs");
        let log_dir = controller.store.paths().logs_dir();
        fs::create_dir_all(&log_dir).unwrap();
        let log_path = log_dir.join("easytier-core.log");
        fs::write(&log_path, "old log line\n").unwrap();

        controller.clear_logs().unwrap();
        assert!(!log_path.exists());
    }

    #[test]
    fn parser_requires_a_single_host_mode() {
        assert!(matches!(
            ServiceOptions::parse(["--service", "--console"]),
            Err(ServiceError::InvalidArguments(_))
        ));
        let options = ServiceOptions::parse([
            "--service",
            "--state-root",
            "C:\\state",
            "--iperf3",
            "C:\\runtime\\iperf3.exe",
        ])
        .unwrap();
        assert_eq!(options.mode, HostMode::Service);
        assert_eq!(options.state_root, Some(PathBuf::from("C:\\state")));
        assert_eq!(
            options.iperf3_executable,
            Some(PathBuf::from("C:\\runtime\\iperf3.exe"))
        );
        assert!(matches!(
            ServiceOptions::parse(["--service", "--owner-sid", "not-a-sid"]),
            Err(ServiceError::InvalidArguments(_))
        ));
    }

    #[test]
    fn core_command_keeps_the_secret_out_of_cli_arguments_and_configures_file_logs() {
        let args = core_command_arguments(
            std::path::Path::new("C:\\ProgramData\\VibeEasyTier\\runtime\\home.toml"),
            std::path::Path::new("C:\\ProgramData\\VibeEasyTier\\logs"),
        );
        let args = args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(!args.iter().any(|argument| argument == "--network-secret"));
        assert!(args
            .windows(2)
            .any(|pair| { pair[0] == "--rpc-portal" && pair[1] == "127.0.0.1:15888" }));
        assert!(args
            .windows(2)
            .any(|pair| { pair[0] == "--rpc-portal-whitelist" && pair[1] == "127.0.0.1/32" }));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--file-log-dir" && pair[1] == "C:\\ProgramData\\VibeEasyTier\\logs"
        }));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--file-log-count" && pair[1] == "5"));
    }

    #[test]
    fn staged_core_check_keeps_the_secret_out_of_cli_arguments() {
        let args = core_config_check_arguments(std::path::Path::new(
            "C:\\ProgramData\\VibeEasyTier\\runtime\\home.validate.toml",
        ));
        let args = args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(!args.iter().any(|argument| argument == "--network-secret"));
        assert!(args.iter().any(|argument| argument == "--check-config"));
    }

    #[test]
    fn core_config_stderr_is_classified_without_leaking_secret_or_path() {
        let secret = "correct-horse-battery-staple";
        let path = r"C:\ProgramData\VibeEasyTier\runtime\office.validate.toml";
        let stderr =
            format!("failed to deserialize network_secret = {secret:?} from {path}: invalid value");
        let failure = CoreConfigCheckFailure::exited(Some(19), stderr.as_bytes());
        let message = failure.to_string();

        assert_eq!(message, "exit code 19; reason=network_identity");
        assert!(!message.contains(secret));
        assert!(!message.contains(path));
    }

    #[test]
    fn core_config_stderr_categories_cover_common_profile_fields() {
        let cases = [
            (
                "invalid network_name in the staged file",
                CoreConfigFailureReason::NetworkIdentity,
            ),
            (
                "invalid ipv4 CIDR prefix length",
                CoreConfigFailureReason::VirtualAddress,
            ),
            (
                "bootstrap peer URI must include a port",
                CoreConfigFailureReason::BootstrapPeer,
            ),
            (
                "data_compress_algo has invalid type",
                CoreConfigFailureReason::CoreOption,
            ),
            (
                "could not deserialize TOML document",
                CoreConfigFailureReason::TomlFormat,
            ),
            ("unrecognised failure", CoreConfigFailureReason::Unknown),
        ];

        for (stderr, expected) in cases {
            assert_eq!(classify_core_config_stderr(stderr.as_bytes()), expected);
        }
    }

    #[test]
    fn core_config_timeout_and_stderr_capture_are_bounded() {
        assert_eq!(
            CoreConfigCheckFailure::timed_out().to_string(),
            "validation timed out"
        );

        let noisy = vec![b'x'; MAX_CORE_CONFIG_STDERR_BYTES + 4096];
        let captured = capture_core_stderr(Cursor::new(noisy)).unwrap();
        assert_eq!(captured.len(), MAX_CORE_CONFIG_STDERR_BYTES);
    }

    #[test]
    fn core_config_failure_rpc_never_returns_raw_stderr() {
        let mut controller = controller("core-config-stderr-redaction");
        controller.config_validator = CoreConfigValidator::Reject(
            "failed network_secret = \"test-secret\" in C:\\ProgramData\\VibeEasyTier\\runtime\\home.validate.toml",
        );

        let response = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        let message = response
            .error
            .expect("validation failure should return an RPC error")
            .message;

        assert_eq!(
            message,
            "easytier-core configuration validation failed: terminated by the operating system; reason=network_identity"
        );
        assert!(!message.contains("test-secret"));
        assert!(!message.contains("C:\\ProgramData"));
        assert!(controller.persisted_state().profiles.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn real_core_config_check_returns_a_sanitized_parse_failure() {
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
            return;
        }

        let secret = "real-core-check-secret";
        let config_path = std::env::temp_dir().join(format!(
            "vibe-easytier-invalid-check-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::write(
            &config_path,
            format!(
                "[network_identity]\nnetwork_name = \"private\"\nnetwork_secret = \"{secret}\"\n[flags\n"
            ),
        )
        .unwrap();

        let error = check_core_config(&core, &config_path)
            .expect_err("malformed TOML must fail Core validation");
        let _ = fs::remove_file(&config_path);
        let message = error.to_string();

        assert!(message.starts_with(CORE_CONFIG_VALIDATION_ERROR_PREFIX));
        assert!(!message.contains(secret));
        assert!(!message.contains(config_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn core_validation_failure_keeps_the_current_durable_connection() {
        let mut controller = controller("failed-core-validation");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::SetConnectionIntent {
                intent: ConnectionIntent::Connect {
                    profile_id: "home".to_owned(),
                },
            },
        ));
        let before = controller.persisted_state().clone();
        controller.config_validator = CoreConfigValidator::Reject("invalid test configuration");

        let mut replacement = profile("home");
        replacement.name = "replacement".to_owned();
        let response = controller.handle_rpc(RpcRequest::new(
            3,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: replacement,
                make_active: true,
            }),
        ));

        assert!(response.error.is_some());
        assert_eq!(controller.persisted_state(), &before);
    }

    #[test]
    fn flag_update_keeps_the_secret_and_restarts_an_active_profile() {
        let mut controller = controller("update-profile-flags");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        let original_secret = controller.persisted_state().profiles["home"]
            .network_secret
            .clone();
        let mut flags = EasyTierFlags::default();
        flags.latency_first = true;
        flags.enable_quic_proxy = true;
        flags.foreign_relay_bps_limit = 42_000;

        let response = controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::UpdateProfileFlags {
                profile_id: "home".to_owned(),
                flags: flags.clone(),
            },
        ));

        let Some(RpcResult::ProfileSaved(view)) = response.result else {
            panic!("expected a profile-save response");
        };
        assert!(response.error.is_none());
        assert_eq!(view.flags, flags);
        assert_eq!(
            controller.persisted_state().profiles["home"].network_secret,
            original_secret
        );
        assert_eq!(
            controller.persisted_state().profiles["home"].flags,
            view.flags
        );
        assert!(controller.force_restart);
    }

    #[test]
    fn rejected_flag_update_does_not_replace_the_old_profile() {
        let mut controller = controller("rejected-profile-flags");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        let before = controller.persisted_state().clone();
        controller.config_validator = CoreConfigValidator::Reject("invalid flag TOML");

        let mut flags = EasyTierFlags::default();
        flags.enable_kcp_proxy = true;
        let response = controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::UpdateProfileFlags {
                profile_id: "home".to_owned(),
                flags,
            },
        ));

        assert!(response.error.is_some());
        assert_eq!(controller.persisted_state(), &before);
    }

    #[test]
    fn flag_update_of_unknown_profile_is_not_created() {
        let mut controller = controller("missing-profile-flags");
        let response = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpdateProfileFlags {
                profile_id: "missing".to_owned(),
                flags: EasyTierFlags::default(),
            },
        ));

        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(RpcErrorCode::NotFound)
        );
        assert!(controller.persisted_state().profiles.is_empty());
    }

    #[test]
    fn blank_device_name_is_resolved_and_persisted_by_profile_upsert() {
        let mut controller = controller("blank-device-name");
        let mut requested = profile("home");
        requested.hostname = "   ".to_owned();

        let response = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: requested,
                make_active: true,
            }),
        ));

        let saved_hostname = match response.result {
            Some(RpcResult::ProfileSaved(profile)) => profile.hostname,
            other => panic!("expected saved profile response, got {other:?}"),
        };
        assert!(response.error.is_none());
        assert!(!saved_hostname.trim().is_empty());
        assert_eq!(
            controller.persisted_state().profiles["home"].hostname,
            saved_hostname
        );
    }

    #[test]
    fn toml_import_uses_the_service_whitelist_and_selects_the_first_profile() {
        let mut controller = controller("toml-import");
        let response = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::ImportProfile {
                toml: r#"
instance_name = "home"
hostname = "laptop"
ipv4 = "10.44.0.2/24"

[[peer]]
uri = "tcp://seed.example.net:11010"

[network_identity]
network_name = "private-network"
network_secret = "test-secret"

[flags]
private_mode = true
enable_encryption = true
accept_dns = false
"#
                .to_owned(),
                make_active: true,
            },
        ));

        assert!(response.error.is_none());
        assert_eq!(
            controller.status().active_profile_id.as_deref(),
            Some("home")
        );
        assert_eq!(controller.persisted_state().profiles.len(), 1);
        assert_eq!(
            controller.persisted_state().profiles["home"].network_name,
            "private-network"
        );
    }

    #[test]
    fn profile_export_returns_a_complete_core_toml_without_mutating_state() {
        let mut controller = controller("toml-export");
        let saved = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        assert!(saved.error.is_none());

        let before = controller.persisted_state().clone();
        let exported = controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::ExportProfile {
                profile_id: "home".to_owned(),
            },
        ));
        let toml = match exported.result {
            Some(RpcResult::ProfileToml { profile_id, toml }) => {
                assert_eq!(profile_id, "home");
                toml
            }
            other => panic!("expected exported profile TOML, got {other:?}"),
        };

        let parsed = toml.parse::<toml::Value>().unwrap();
        assert_eq!(
            parsed["network_identity"]["network_name"].as_str(),
            Some("private-network")
        );
        assert_eq!(
            parsed["network_identity"]["network_secret"].as_str(),
            Some("test-secret")
        );
        assert_eq!(controller.persisted_state(), &before);
    }

    #[test]
    fn exporting_an_unknown_profile_is_a_not_found_error() {
        let mut controller = controller("toml-export-missing");
        let response = controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::ExportProfile {
                profile_id: "missing".to_owned(),
            },
        ));

        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(RpcErrorCode::NotFound)
        );
    }

    #[test]
    fn status_marks_unknown_peers_separately_from_zero_peers() {
        let mut controller = controller("peer-status");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("home"),
                make_active: true,
            }),
        ));
        controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::SetConnectionIntent {
                intent: ConnectionIntent::Connect {
                    profile_id: "home".to_owned(),
                },
            },
        ));
        controller.reconcile(1, true);
        controller.on_core_started(99, 2);
        assert_eq!(
            controller.status().state,
            ServiceConnectionState::Connecting,
            "a spawned core is not a connected private network before a remote peer is observed"
        );
        controller.on_health_sample(
            HealthSample {
                core_process_running: true,
                control_plane_healthy: true,
                private_network_reachable: None,
                connected_peer_count: Some(0),
                connected_peers: Some(Vec::new()),
                route_count: Some(1),
                traffic_tx_bytes: Some(10),
                traffic_rx_bytes: Some(20),
            },
            3,
        );
        let status = controller.status();
        assert_eq!(status.peer_count, 0);
        assert!(status.peer_count_available);
        assert_eq!(status.last_success_unix_ms, Some(3));
        assert_eq!(status.state, ServiceConnectionState::Connecting);

        controller.on_health_sample(
            HealthSample {
                core_process_running: true,
                control_plane_healthy: true,
                private_network_reachable: None,
                connected_peer_count: None,
                connected_peers: None,
                route_count: None,
                traffic_tx_bytes: None,
                traffic_rx_bytes: None,
            },
            4,
        );
        let status = controller.status();
        assert_eq!(status.peer_count, 0);
        assert!(!status.peer_count_available);
        assert_eq!(status.route_count, 1);
        assert_eq!(status.traffic_tx_bytes, 10);
        assert_eq!(status.traffic_rx_bytes, 20);
        assert_eq!(status.last_success_unix_ms, Some(4));
        assert_eq!(status.state, ServiceConnectionState::Connecting);

        controller.on_health_sample(
            HealthSample {
                core_process_running: true,
                control_plane_healthy: true,
                private_network_reachable: None,
                connected_peer_count: Some(1),
                connected_peers: Some(vec![ConnectedPeer {
                    id: "remote".to_owned(),
                    hostname: "remote-node".to_owned(),
                    ipv4: "10.44.0.3".to_owned(),
                    cidr: Some("10.44.0.3/24".to_owned()),
                    cost: Some("10".to_owned()),
                    latency_ms: Some(12),
                    rx_bytes: None,
                    tx_bytes: None,
                    protocols: Vec::new(),
                    tunnel_protocol: None,
                    nat_type: None,
                    version: None,
                }]),
                route_count: Some(2),
                traffic_tx_bytes: Some(30),
                traffic_rx_bytes: Some(40),
            },
            5,
        );
        let status = controller.status();
        assert_eq!(status.state, ServiceConnectionState::Connected);
        assert_eq!(status.route_count, 2);
        assert_eq!(status.traffic_tx_bytes, 30);
        assert_eq!(status.traffic_rx_bytes, 40);
    }

    #[test]
    fn cli_peer_json_parser_accepts_known_list_shapes_without_guessing() {
        assert_eq!(peer_count_from_cli_json("[]"), Some(0));
        assert_eq!(peer_count_from_cli_json(r#"{"peers":[{},{}]}"#), Some(2));
        assert_eq!(
            peer_count_from_cli_json(r#"{"data":{"peer_infos":[{}]}}"#),
            Some(1)
        );
        assert_eq!(peer_count_from_cli_json(r#"{"unexpected":[]}"#), None);
    }

    #[test]
    fn cli_peer_json_parser_sanitizes_peer_rows() {
        let peers = peers_from_cli_json(
            r#"[
              {
                "cidr": "10.251.0.1/24",
                "ipv4": "10.251.0.1",
                "hostname": "probe-one",
                "cost": "Local",
                "lat_ms": "12.6",
                "rx_bytes": "123",
                "tx_bytes": "456",
                "tunnel_proto": "tcp",
                "nat_type": "Unknown",
                "id": "2857754789",
                "version": "2.6.4-8428a89d"
              }
            ]"#,
        )
        .unwrap();

        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, "2857754789");
        assert_eq!(peers[0].hostname, "probe-one");
        assert_eq!(peers[0].ipv4, "10.251.0.1");
        assert_eq!(peers[0].latency_ms, Some(13));
        assert_eq!(peers[0].rx_bytes, Some(123));
        assert_eq!(peers[0].tx_bytes, Some(456));
        assert_eq!(peers[0].protocols, vec!["tcp"]);
        assert_eq!(peers[0].tunnel_protocol.as_deref(), Some("tcp"));
    }

    #[test]
    fn cli_peer_json_parser_accepts_human_readable_traffic_units() {
        let peers = peers_from_cli_json(
            r#"[{"id":"remote","hostname":"remote","ipv4":"10.44.0.3","rx_bytes":"122.65 MB","tx_bytes":"9.89 MB"}]"#,
        )
        .unwrap();

        assert_eq!(peers[0].rx_bytes, Some(128_607_846));
        assert_eq!(peers[0].tx_bytes, Some(10_370_417));
        assert_eq!(parse_human_bytes("1.5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_human_bytes("-"), None);
    }

    #[test]
    fn cli_route_and_stats_parsers_use_live_core_metrics() {
        assert_eq!(
            route_count_from_cli_json(r#"[{"ipv4":"local"},{"ipv4":"remote"}]"#),
            Some(2)
        );
        assert_eq!(
            route_count_from_cli_json(r#"{"data":{"routes":[{}, {}, {}]}}"#),
            Some(3)
        );
        assert_eq!(route_count_from_cli_json(r#"{"unknown":[]}"#), None);

        let traffic = traffic_from_cli_json(
            r#"[
              {"name":"traffic_bytes_tx","value":999},
              {"name":"traffic_bytes_rx","value":"888"},
              {"name":"traffic_bytes_self_tx","value":10802127},
              {"name":"traffic_bytes_self_rx","value":122278646}
            ]"#,
        )
        .unwrap();
        assert_eq!(
            traffic,
            CoreTraffic {
                tx_bytes: 10_802_127,
                rx_bytes: 122_278_646,
            }
        );

        let fallback = traffic_from_cli_json(
            r#"{"stats":[{"name":"traffic_bytes_tx","value":42},{"name":"traffic_bytes_rx","value":84}]}"#,
        )
        .unwrap();
        assert_eq!(
            fallback,
            CoreTraffic {
                tx_bytes: 42,
                rx_bytes: 84
            }
        );
    }

    #[test]
    fn cli_peer_json_parser_exposes_multiple_sanitized_protocols_per_peer() {
        let peers = peers_from_cli_json(
            r#"[
              {
                "id": "remote",
                "hostname": "remote-node",
                "ipv4": "10.44.0.3",
                "tunnel_proto": " TCP, wg, tcp6, bad://secret, WSS "
              }
            ]"#,
        )
        .unwrap();

        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].protocols, vec!["tcp", "wg", "tcp6", "wss"]);
        assert_eq!(peers[0].tunnel_protocol.as_deref(), Some("tcp,wg,tcp6,wss"));
    }

    #[test]
    fn cli_peer_json_parser_accepts_a_future_protocol_array_and_merges_duplicate_rows() {
        let peers = peers_from_cli_json(
            r#"{
              "peers": [
                {
                  "id": "remote",
                  "hostname": "first-name-wins",
                  "ipv4": "10.44.0.3",
                  "tunnel_protocols": ["udp", "wg", "unsafe:value"]
                },
                {
                  "id": "remote",
                  "hostname": "must-not-replace",
                  "ipv4": "10.44.0.4",
                  "tunnel_proto": "tcp,wg,quic"
                }
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].hostname, "first-name-wins");
        assert_eq!(peers[0].ipv4, "10.44.0.3");
        assert_eq!(peers[0].protocols, vec!["udp", "wg", "tcp", "quic"]);
        assert_eq!(peers[0].tunnel_protocol.as_deref(), Some("udp,wg,tcp,quic"));
    }

    #[test]
    fn profile_views_are_stably_sorted_by_id() {
        let mut controller = controller("profile-order");
        controller.handle_rpc(RpcRequest::new(
            1,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("zebra"),
                make_active: false,
            }),
        ));
        controller.handle_rpc(RpcRequest::new(
            2,
            RpcCommand::UpsertProfile(ProfileUpsert {
                profile: profile("alpha"),
                make_active: false,
            }),
        ));

        let ids = controller
            .profile_views()
            .into_iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["alpha", "zebra"]);
    }

    #[test]
    fn long_loop_gap_is_treated_as_a_resume_signal() {
        let poll = Duration::from_secs(1);
        assert!(!is_resume_gap(Duration::from_secs(3), poll));
        assert!(is_resume_gap(Duration::from_millis(3_001), poll));
    }
}
