//! Deterministic process-supervision policy for the EasyTier core child.
//!
//! The service host supplies time and observations; this module decides when
//! to start, stop, probe, and restart.  Keeping that policy pure makes the
//! reconnect promise testable without Windows-specific process APIs.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::protocol::ConnectedPeer;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorState {
    Stopped,
    WaitingForNetwork,
    Starting,
    Running,
    BackingOff,
    Degraded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorAction {
    Noop,
    StartCore { profile_id: String },
    StopCore,
    ProbeHealth,
    RestartCore { profile_id: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthSample {
    pub core_process_running: bool,
    pub control_plane_healthy: bool,
    /// `None` means the profile has no separate reachability probe configured.
    pub private_network_reachable: Option<bool>,
    /// A successful `easytier-cli peer list` observation. `None` deliberately
    /// means that peer data could not be obtained; it is not a claim that the
    /// private network currently has zero remote peers.
    pub connected_peer_count: Option<usize>,
    /// Sanitized rows from the same CLI observation that produced
    /// `connected_peer_count`. `None` means the CLI output was unavailable or
    /// could not be parsed, while `Some(vec![])` is a confirmed empty list.
    pub connected_peers: Option<Vec<ConnectedPeer>>,
    /// Number of entries returned by `easytier-cli route list`.
    pub route_count: Option<usize>,
    /// Host-originated payload counters from EasyTier's statistics registry.
    pub traffic_tx_bytes: Option<u64>,
    pub traffic_rx_bytes: Option<u64>,
}

impl HealthSample {
    pub fn healthy(&self) -> bool {
        self.core_process_running
            && self.control_plane_healthy
            && self.private_network_reachable.unwrap_or(true)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub initial_backoff: Duration,
    pub maximum_backoff: Duration,
    pub health_probe_interval: Duration,
    pub stable_connection_window: Duration,
    pub max_consecutive_failures_before_degraded: u32,
    pub max_consecutive_health_failures: u32,
    /// A healthy core with no remote peers is degraded connectivity, not a
    /// hung core. Give it time to converge before one conservative restart.
    pub no_peer_restart_after: Duration,
    pub no_peer_restart_min_interval: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(2),
            maximum_backoff: Duration::from_secs(5 * 60),
            health_probe_interval: Duration::from_secs(15),
            stable_connection_window: Duration::from_secs(60),
            max_consecutive_failures_before_degraded: 5,
            max_consecutive_health_failures: 3,
            no_peer_restart_after: Duration::from_secs(10 * 60),
            no_peer_restart_min_interval: Duration::from_secs(15 * 60),
        }
    }
}

/// State machine for a single selected profile.
#[derive(Clone, Debug)]
pub struct Supervisor {
    profile_id: String,
    desired: bool,
    network_available: bool,
    state: SupervisorState,
    policy: RetryPolicy,
    consecutive_start_failures: u32,
    consecutive_health_failures: u32,
    retry_at_ms: Option<u64>,
    started_at_ms: Option<u64>,
    next_health_probe_at_ms: Option<u64>,
    no_peer_since_ms: Option<u64>,
    last_no_peer_restart_ms: Option<u64>,
}

impl Supervisor {
    pub fn new(profile_id: impl Into<String>, desired: bool, policy: RetryPolicy) -> Self {
        Self {
            profile_id: profile_id.into(),
            desired,
            network_available: false,
            state: SupervisorState::Stopped,
            policy,
            consecutive_start_failures: 0,
            consecutive_health_failures: 0,
            retry_at_ms: None,
            started_at_ms: None,
            next_health_probe_at_ms: None,
            no_peer_since_ms: None,
            last_no_peer_restart_ms: None,
        }
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn state(&self) -> SupervisorState {
        self.state
    }

    pub fn desired(&self) -> bool {
        self.desired
    }

    pub fn retry_at_ms(&self) -> Option<u64> {
        self.retry_at_ms
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_start_failures
            .max(self.consecutive_health_failures)
    }

    /// Called once when the service process begins managing this profile.
    pub fn initialize(&mut self, now_ms: u64, network_available: bool) -> SupervisorAction {
        self.network_available = network_available;
        self.reconcile_desired_state(now_ms)
    }

    /// Explicit user intent always wins over automatic recovery.
    pub fn set_desired(&mut self, desired: bool, now_ms: u64) -> SupervisorAction {
        self.desired = desired;
        self.reconcile_desired_state(now_ms)
    }

    pub fn on_network_changed(&mut self, network_available: bool, now_ms: u64) -> SupervisorAction {
        self.network_available = network_available;
        self.reconcile_desired_state(now_ms)
    }

    /// Resume has a useful fast path: probe an existing child immediately, or
    /// start recovery right away if the core had stopped while suspended.
    pub fn on_system_resume(&mut self, now_ms: u64, network_available: bool) -> SupervisorAction {
        self.network_available = network_available;
        if self.state == SupervisorState::Running && self.desired && network_available {
            self.next_health_probe_at_ms = Some(now_ms);
            return SupervisorAction::ProbeHealth;
        }
        self.reconcile_desired_state(now_ms)
    }

    pub fn on_core_started(&mut self, now_ms: u64) -> SupervisorAction {
        if !self.desired {
            self.reset_to_stopped();
            return SupervisorAction::StopCore;
        }
        if !self.network_available {
            self.reset_to_waiting();
            return SupervisorAction::StopCore;
        }

        self.state = SupervisorState::Running;
        self.retry_at_ms = None;
        self.started_at_ms = Some(now_ms);
        self.next_health_probe_at_ms =
            Some(add_duration(now_ms, self.policy.health_probe_interval));
        self.consecutive_health_failures = 0;
        SupervisorAction::Noop
    }

    pub fn on_core_start_failed(&mut self, now_ms: u64) {
        if !self.desired {
            self.reset_to_stopped();
            return;
        }
        if !self.network_available {
            self.reset_to_waiting();
            return;
        }
        self.schedule_backoff(now_ms);
    }

    pub fn on_core_exited(&mut self, now_ms: u64) {
        if !self.desired {
            self.reset_to_stopped();
            return;
        }
        if !self.network_available {
            self.reset_to_waiting();
            return;
        }
        self.schedule_backoff(now_ms);
    }

    pub fn on_health_sample(&mut self, sample: HealthSample, now_ms: u64) -> SupervisorAction {
        if !self.desired || self.state != SupervisorState::Running {
            return SupervisorAction::Noop;
        }
        if sample.healthy() {
            self.consecutive_health_failures = 0;
            return self.on_healthy_peer_observation(sample.connected_peer_count, now_ms);
        }

        if !sample.core_process_running {
            self.on_core_exited(now_ms);
            return SupervisorAction::Noop;
        }

        self.consecutive_health_failures = self.consecutive_health_failures.saturating_add(1);
        if self.consecutive_health_failures < self.policy.max_consecutive_health_failures.max(1) {
            return SupervisorAction::Noop;
        }

        self.state = SupervisorState::Starting;
        self.started_at_ms = None;
        self.next_health_probe_at_ms = None;
        self.retry_at_ms = None;
        SupervisorAction::RestartCore {
            profile_id: self.profile_id.clone(),
        }
    }

    /// Advances timers and returns at most one action for the service host.
    pub fn tick(&mut self, now_ms: u64) -> SupervisorAction {
        if !self.desired {
            return SupervisorAction::Noop;
        }
        if !self.network_available {
            return SupervisorAction::Noop;
        }

        match self.state {
            SupervisorState::Stopped | SupervisorState::WaitingForNetwork => self.begin_start(),
            SupervisorState::BackingOff | SupervisorState::Degraded => {
                if self.retry_at_ms.is_some_and(|retry_at| now_ms >= retry_at) {
                    self.begin_start()
                } else {
                    SupervisorAction::Noop
                }
            }
            SupervisorState::Starting => SupervisorAction::Noop,
            SupervisorState::Running => {
                if self.started_at_ms.is_some_and(|started_at| {
                    now_ms.saturating_sub(started_at)
                        >= self
                            .policy
                            .stable_connection_window
                            .as_millis()
                            .min(u128::from(u64::MAX)) as u64
                }) {
                    self.consecutive_start_failures = 0;
                }
                if self
                    .next_health_probe_at_ms
                    .is_some_and(|next_probe| now_ms >= next_probe)
                {
                    self.next_health_probe_at_ms =
                        Some(add_duration(now_ms, self.policy.health_probe_interval));
                    SupervisorAction::ProbeHealth
                } else {
                    SupervisorAction::Noop
                }
            }
        }
    }

    fn reconcile_desired_state(&mut self, now_ms: u64) -> SupervisorAction {
        if !self.desired {
            let must_stop = !matches!(self.state, SupervisorState::Stopped);
            self.reset_to_stopped();
            return if must_stop {
                SupervisorAction::StopCore
            } else {
                SupervisorAction::Noop
            };
        }

        if !self.network_available {
            let must_stop = matches!(
                self.state,
                SupervisorState::Starting | SupervisorState::Running
            );
            self.reset_to_waiting();
            return if must_stop {
                SupervisorAction::StopCore
            } else {
                SupervisorAction::Noop
            };
        }

        match self.state {
            SupervisorState::Running | SupervisorState::Starting => SupervisorAction::Noop,
            SupervisorState::BackingOff | SupervisorState::Degraded => {
                if self.retry_at_ms.is_some_and(|retry_at| now_ms >= retry_at) {
                    self.begin_start()
                } else {
                    SupervisorAction::Noop
                }
            }
            SupervisorState::Stopped | SupervisorState::WaitingForNetwork => self.begin_start(),
        }
    }

    fn begin_start(&mut self) -> SupervisorAction {
        self.state = SupervisorState::Starting;
        self.retry_at_ms = None;
        self.started_at_ms = None;
        self.next_health_probe_at_ms = None;
        SupervisorAction::StartCore {
            profile_id: self.profile_id.clone(),
        }
    }

    fn schedule_backoff(&mut self, now_ms: u64) {
        self.consecutive_start_failures = self.consecutive_start_failures.saturating_add(1);
        let delay = self.backoff_delay();
        self.retry_at_ms = Some(add_duration(now_ms, delay));
        self.started_at_ms = None;
        self.next_health_probe_at_ms = None;
        self.state = if self.consecutive_start_failures
            >= self.policy.max_consecutive_failures_before_degraded.max(1)
        {
            SupervisorState::Degraded
        } else {
            SupervisorState::BackingOff
        };
    }

    fn backoff_delay(&self) -> Duration {
        let shift = self.consecutive_start_failures.saturating_sub(1).min(31);
        let initial_ms = self
            .policy
            .initial_backoff
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let max_ms = self
            .policy
            .maximum_backoff
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
        let exponential_ms = initial_ms.saturating_mul(multiplier).min(max_ms);
        // Keep the upper bound strict while spreading repeated recovery work
        // across machines. The profile id and failure count make this stable
        // enough to unit test yet different for normal UUID-backed profiles.
        let lower_bound_ms = exponential_ms.saturating_mul(80) / 100;
        let spread_ms = exponential_ms.saturating_sub(lower_bound_ms);
        let jitter = deterministic_jitter(&self.profile_id, self.consecutive_start_failures)
            % spread_ms.saturating_add(1);
        Duration::from_millis(lower_bound_ms.saturating_add(jitter))
    }

    fn reset_to_stopped(&mut self) {
        self.state = SupervisorState::Stopped;
        self.retry_at_ms = None;
        self.started_at_ms = None;
        self.next_health_probe_at_ms = None;
        self.consecutive_health_failures = 0;
        self.no_peer_since_ms = None;
    }

    fn reset_to_waiting(&mut self) {
        self.state = SupervisorState::WaitingForNetwork;
        self.retry_at_ms = None;
        self.started_at_ms = None;
        self.next_health_probe_at_ms = None;
        self.consecutive_health_failures = 0;
        self.no_peer_since_ms = None;
    }

    fn on_healthy_peer_observation(
        &mut self,
        connected_peer_count: Option<usize>,
        now_ms: u64,
    ) -> SupervisorAction {
        let Some(peer_count) = connected_peer_count else {
            // Do not turn a telemetry failure into a no-peer verdict.
            return SupervisorAction::Noop;
        };
        if peer_count > 0 {
            self.no_peer_since_ms = None;
            return SupervisorAction::Noop;
        }

        let no_peer_since = self.no_peer_since_ms.get_or_insert(now_ms);
        let no_peer_long_enough =
            now_ms.saturating_sub(*no_peer_since) >= duration_ms(self.policy.no_peer_restart_after);
        let restart_rate_limited = self.last_no_peer_restart_ms.is_some_and(|last_restart| {
            now_ms.saturating_sub(last_restart)
                < duration_ms(self.policy.no_peer_restart_min_interval)
        });
        if !self.network_available || !no_peer_long_enough || restart_rate_limited {
            return SupervisorAction::Noop;
        }

        self.last_no_peer_restart_ms = Some(now_ms);
        self.state = SupervisorState::Starting;
        self.started_at_ms = None;
        self.next_health_probe_at_ms = None;
        self.retry_at_ms = None;
        SupervisorAction::RestartCore {
            profile_id: self.profile_id.clone(),
        }
    }
}

fn deterministic_jitter(profile_id: &str, failures: u32) -> u64 {
    // FNV-1a gives a small dependency-free, repeatable jitter source. It is
    // intentionally not used for secrets or security decisions.
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for byte in profile_id.bytes().chain(failures.to_le_bytes()) {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    value
}

fn add_duration(now_ms: u64, duration: Duration) -> u64 {
    now_ms.saturating_add(duration_ms(duration))
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RetryPolicy {
        RetryPolicy {
            initial_backoff: Duration::from_millis(10),
            maximum_backoff: Duration::from_millis(40),
            health_probe_interval: Duration::from_millis(5),
            stable_connection_window: Duration::from_millis(20),
            max_consecutive_failures_before_degraded: 3,
            max_consecutive_health_failures: 2,
            no_peer_restart_after: Duration::from_millis(10),
            no_peer_restart_min_interval: Duration::from_millis(15),
        }
    }

    #[test]
    fn boot_starts_the_durable_auto_connect_profile() {
        let mut supervisor = Supervisor::new("home", true, policy());

        assert_eq!(
            supervisor.initialize(0, true),
            SupervisorAction::StartCore {
                profile_id: "home".to_owned()
            }
        );
        assert_eq!(supervisor.state(), SupervisorState::Starting);
        assert_eq!(supervisor.on_core_started(1), SupervisorAction::Noop);
        assert_eq!(supervisor.state(), SupervisorState::Running);
    }

    #[test]
    fn failed_starts_back_off_and_retry() {
        let mut supervisor = Supervisor::new("home", true, policy());
        supervisor.initialize(0, true);
        supervisor.on_core_start_failed(1);

        assert_eq!(supervisor.state(), SupervisorState::BackingOff);
        let retry_at = supervisor.retry_at_ms().unwrap();
        assert!((9..=11).contains(&retry_at));
        assert_eq!(
            supervisor.tick(retry_at.saturating_sub(1)),
            SupervisorAction::Noop
        );
        assert!(matches!(
            supervisor.tick(retry_at),
            SupervisorAction::StartCore { .. }
        ));
    }

    #[test]
    fn default_backoff_is_jittered_and_never_exceeds_five_minutes() {
        let mut home = Supervisor::new("home", true, RetryPolicy::default());
        let mut office = Supervisor::new("office", true, RetryPolicy::default());
        home.initialize(0, true);
        office.initialize(0, true);

        for now_ms in 1..=40 {
            home.on_core_start_failed(now_ms);
            office.on_core_start_failed(now_ms);
        }

        let home_delay = home.retry_at_ms().unwrap().saturating_sub(40);
        let office_delay = office.retry_at_ms().unwrap().saturating_sub(40);
        assert!(home_delay <= Duration::from_secs(5 * 60).as_millis() as u64);
        assert!(office_delay <= Duration::from_secs(5 * 60).as_millis() as u64);
        assert_ne!(home_delay, office_delay);
    }

    #[test]
    fn explicit_disconnect_prevents_automatic_relaunch() {
        let mut supervisor = Supervisor::new("home", true, policy());
        supervisor.initialize(0, true);
        supervisor.on_core_started(1);

        assert_eq!(supervisor.set_desired(false, 2), SupervisorAction::StopCore);
        supervisor.on_core_exited(3);
        assert_eq!(supervisor.tick(100), SupervisorAction::Noop);
        assert_eq!(supervisor.state(), SupervisorState::Stopped);
    }

    #[test]
    fn unhealthy_running_core_is_restarted_after_the_threshold() {
        let mut supervisor = Supervisor::new("home", true, policy());
        supervisor.initialize(0, true);
        supervisor.on_core_started(1);
        let unhealthy = HealthSample {
            core_process_running: true,
            control_plane_healthy: false,
            private_network_reachable: Some(false),
            connected_peer_count: None,
            connected_peers: None,
            route_count: None,
            traffic_tx_bytes: None,
            traffic_rx_bytes: None,
        };

        assert_eq!(
            supervisor.on_health_sample(unhealthy.clone(), 2),
            SupervisorAction::Noop
        );
        assert_eq!(
            supervisor.on_health_sample(unhealthy, 3),
            SupervisorAction::RestartCore {
                profile_id: "home".to_owned()
            }
        );
    }

    #[test]
    fn network_loss_stops_core_and_resume_restarts_it() {
        let mut supervisor = Supervisor::new("home", true, policy());
        supervisor.initialize(0, true);
        supervisor.on_core_started(1);

        assert_eq!(
            supervisor.on_network_changed(false, 2),
            SupervisorAction::StopCore
        );
        assert_eq!(supervisor.state(), SupervisorState::WaitingForNetwork);
        assert!(matches!(
            supervisor.on_system_resume(3, true),
            SupervisorAction::StartCore { .. }
        ));
    }

    #[test]
    fn no_peer_recovery_is_conservative_and_rate_limited() {
        let mut supervisor = Supervisor::new("home", true, policy());
        supervisor.initialize(0, true);
        supervisor.on_core_started(1);
        let no_peers = HealthSample {
            core_process_running: true,
            control_plane_healthy: true,
            private_network_reachable: None,
            connected_peer_count: Some(0),
            connected_peers: Some(Vec::new()),
            route_count: Some(1),
            traffic_tx_bytes: Some(0),
            traffic_rx_bytes: Some(0),
        };

        assert_eq!(
            supervisor.on_health_sample(no_peers.clone(), 2),
            SupervisorAction::Noop
        );
        assert_eq!(
            supervisor.on_health_sample(no_peers.clone(), 11),
            SupervisorAction::Noop
        );
        assert_eq!(
            supervisor.on_health_sample(no_peers.clone(), 12),
            SupervisorAction::RestartCore {
                profile_id: "home".to_owned()
            }
        );
        supervisor.on_core_started(13);
        assert_eq!(
            supervisor.on_health_sample(no_peers, 20),
            SupervisorAction::Noop
        );
    }
}
