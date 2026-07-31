//! Best-effort Windows network availability monitoring.
//!
//! EasyTier can work on a LAN without public Internet access, so the service
//! uses Windows' aggregate connectivity hint rather than an Internet ping.
//! A probe failure deliberately preserves the previous observation: a
//! transient NLM/IP Helper failure must not stop an otherwise healthy tunnel.

use thiserror::Error;

/// Error returned when the operating system cannot provide a connectivity
/// observation. Callers should retain the last successful observation.
#[derive(Debug, Error)]
#[error("network availability probe failed: {message}")]
pub struct NetworkProbeError {
    message: String,
}

impl NetworkProbeError {
    #[cfg(windows)]
    fn windows(error: u32) -> Self {
        Self {
            message: format!("GetNetworkConnectivityHint returned Windows error {error}"),
        }
    }
}

/// Polling monitor that reports only confirmed state transitions.
#[derive(Clone, Debug)]
pub struct NetworkMonitor {
    available: bool,
}

impl NetworkMonitor {
    /// Initializes from Windows' current connectivity hint. If Windows has not
    /// finished initializing networking yet, start optimistically and let the
    /// core's health/backoff policy perform recovery rather than deadlocking
    /// automatic connection at boot.
    pub fn new() -> Self {
        Self::with_initial(probe_network_available().unwrap_or(true))
    }

    /// Creates a monitor with a known initial observation. This is useful for
    /// deterministic hosts and tests.
    pub const fn with_initial(available: bool) -> Self {
        Self { available }
    }

    pub const fn available(&self) -> bool {
        self.available
    }

    /// Refreshes the operating-system observation. `Ok(Some(...))` denotes a
    /// confirmed transition; errors leave the previous value untouched.
    pub fn refresh(&mut self) -> Result<Option<bool>, NetworkProbeError> {
        Ok(self.observe(probe_network_available()?))
    }

    /// Records a supplied observation and returns it only when it changed.
    pub fn observe(&mut self, available: bool) -> Option<bool> {
        if self.available == available {
            return None;
        }
        self.available = available;
        Some(available)
    }
}

/// Returns whether Windows reports usable local or Internet network access.
///
/// `LocalAccess` is intentionally treated as available because a direct
/// EasyTier peer or a private relay may be reachable on a network without
/// public Internet access.
pub fn probe_network_available() -> Result<bool, NetworkProbeError> {
    #[cfg(windows)]
    {
        use std::mem::MaybeUninit;

        use windows_sys::Win32::{
            NetworkManagement::IpHelper::GetNetworkConnectivityHint,
            Networking::WinSock::NL_NETWORK_CONNECTIVITY_HINT,
        };

        // The function fills every field in this POD C structure on success.
        let mut hint = MaybeUninit::<NL_NETWORK_CONNECTIVITY_HINT>::zeroed();
        let result = unsafe { GetNetworkConnectivityHint(hint.as_mut_ptr()) };
        if result != 0 {
            return Err(NetworkProbeError::windows(result));
        }
        let hint = unsafe { hint.assume_init() };

        // 0 = unknown, 1 = none, 2 = local, 3 = Internet, 4 = constrained
        // Internet, 5 = hidden. Captive-portal/local access can still reach a
        // private EasyTier peer, so keep 2 through 4 eligible for recovery.
        return Ok((2..=4).contains(&hint.ConnectivityLevel));
    }

    #[cfg(not(windows))]
    {
        // The service host is Windows-only. Keeping this API usable in unit
        // tests on other platforms avoids inventing a second policy there.
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::NetworkMonitor;

    #[test]
    fn monitor_only_emits_confirmed_transitions() {
        let mut monitor = NetworkMonitor::with_initial(true);

        assert_eq!(monitor.observe(true), None);
        assert_eq!(monitor.observe(false), Some(false));
        assert!(!monitor.available());
        assert_eq!(monitor.observe(false), None);
        assert_eq!(monitor.observe(true), Some(true));
        assert!(monitor.available());
    }
}
