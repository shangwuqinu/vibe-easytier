//! Bounded iperf3 probes over the active EasyTier virtual interface.
//!
//! The service supervises one bundled iperf3 server bound to the active
//! profile's virtual IPv4 address. Desktop-initiated clients bind to the same
//! virtual interface, so tests cannot silently fall back to a physical route.

use std::{
    ffi::OsString,
    io::{self, Read},
    net::Ipv4Addr,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

pub const BANDWIDTH_TEST_PORT: u16 = 29_999;
pub const DEFAULT_TEST_DURATION_SECONDS: u8 = 3;
const MIN_TEST_DURATION_SECONDS: u8 = 1;
const MAX_TEST_DURATION_SECONDS: u8 = 10;
const OMIT_SECONDS: u8 = 1;
const CONNECT_TIMEOUT_MILLISECONDS: u32 = 4_000;
const MAX_IPERF_OUTPUT_BYTES: u64 = 1024 * 1024;
const SERVER_RESTART_DELAY: Duration = Duration::from_secs(2);
const SERVER_MAX_DURATION_SECONDS: u32 = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Iperf3BindTarget {
    pub address: Ipv4Addr,
}

impl Iperf3BindTarget {
    pub fn from_cidr(cidr: &str) -> Option<Self> {
        let (address, prefix_length) = cidr.trim().split_once('/')?;
        let address = address.parse().ok()?;
        let prefix_length = prefix_length.parse::<u8>().ok()?;
        (prefix_length <= 32).then_some(Self { address })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BandwidthMeasurement {
    pub download_bps: u64,
    pub upload_bps: u64,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub duration_seconds: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirectionMeasurement {
    bits_per_second: u64,
    bytes: u64,
}

/// Runs a normal iperf3 client test for upload and a reverse test for
/// download. The directions are sequential so they do not compete with each
/// other for the same EasyTier path.
pub fn run_iperf3_test(
    executable: &Path,
    local_address: Ipv4Addr,
    peer_address: Ipv4Addr,
    duration_seconds: u8,
) -> io::Result<BandwidthMeasurement> {
    validate_duration(duration_seconds)?;
    if local_address == peer_address {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "iperf3 peer address is the local virtual address",
        ));
    }

    let upload = run_direction_with_retry(
        executable,
        local_address,
        peer_address,
        duration_seconds,
        false,
    )?;
    let download = run_direction_with_retry(
        executable,
        local_address,
        peer_address,
        duration_seconds,
        true,
    )?;

    Ok(BandwidthMeasurement {
        download_bps: download.bits_per_second,
        upload_bps: upload.bits_per_second,
        download_bytes: download.bytes,
        upload_bytes: upload.bytes,
        duration_seconds,
    })
}

fn run_direction_with_retry(
    executable: &Path,
    local_address: Ipv4Addr,
    peer_address: Ipv4Addr,
    duration_seconds: u8,
    reverse: bool,
) -> io::Result<DirectionMeasurement> {
    let mut last_error = None;
    for attempt in 0..3 {
        match run_iperf3_direction(
            executable,
            local_address,
            peer_address,
            duration_seconds,
            reverse,
        ) {
            Ok(measurement) => return Ok(measurement),
            Err(error)
                if attempt < 2
                    && matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotConnected
                    ) =>
            {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("iperf3 client retry failed")))
}

fn run_iperf3_direction(
    executable: &Path,
    local_address: Ipv4Addr,
    peer_address: Ipv4Addr,
    duration_seconds: u8,
    reverse: bool,
) -> io::Result<DirectionMeasurement> {
    let arguments = iperf3_client_arguments(
        local_address,
        peer_address,
        BANDWIDTH_TEST_PORT,
        duration_seconds,
        reverse,
    );
    let timeout = Duration::from_secs(u64::from(duration_seconds) + u64::from(OMIT_SECONDS) + 10);
    let output = run_iperf3_command(executable, &arguments, timeout)?;
    parse_iperf3_json(&output)
}

fn iperf3_client_arguments(
    local_address: Ipv4Addr,
    peer_address: Ipv4Addr,
    port: u16,
    duration_seconds: u8,
    reverse: bool,
) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("--client"),
        OsString::from(peer_address.to_string()),
        OsString::from("--bind"),
        OsString::from(local_address.to_string()),
        OsString::from("--port"),
        OsString::from(port.to_string()),
        OsString::from("--time"),
        OsString::from(duration_seconds.to_string()),
        OsString::from("--omit"),
        OsString::from(OMIT_SECONDS.to_string()),
        OsString::from("--interval"),
        OsString::from("0"),
        OsString::from("--connect-timeout"),
        OsString::from(CONNECT_TIMEOUT_MILLISECONDS.to_string()),
        OsString::from("--json"),
        OsString::from("--version4"),
    ];
    if reverse {
        arguments.push(OsString::from("--reverse"));
    }
    arguments
}

fn run_iperf3_command(
    executable: &Path,
    arguments: &[OsString],
    timeout: Duration,
) -> io::Result<Vec<u8>> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut command);

    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .expect("iperf3 stdout was explicitly configured as piped");
    let stderr = child
        .stderr
        .take()
        .expect("iperf3 stderr was explicitly configured as piped");
    let stdout_reader = read_capped(stdout);
    let stderr_reader = read_capped(stderr);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "iperf3 client timed out",
                ));
            }
        }
    };

    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    if stdout.len() > MAX_IPERF_OUTPUT_BYTES as usize
        || stderr.len() > MAX_IPERF_OUTPUT_BYTES as usize
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "iperf3 output exceeded the allowed size",
        ));
    }
    if !status.success() {
        return Err(classify_iperf3_failure(status, &stdout, &stderr));
    }
    Ok(stdout)
}

fn read_capped<R>(reader: R) -> thread::JoinHandle<io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader
            .take(MAX_IPERF_OUTPUT_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn join_reader(reader: thread::JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other("iperf3 output reader stopped unexpectedly"))?
}

fn classify_iperf3_failure(status: ExitStatus, stdout: &[u8], stderr: &[u8]) -> io::Error {
    let mut diagnostic = String::from_utf8_lossy(stdout).to_lowercase();
    diagnostic.push_str(&String::from_utf8_lossy(stderr).to_lowercase());
    let kind = if diagnostic.contains("server is busy") {
        io::ErrorKind::WouldBlock
    } else if diagnostic.contains("connection refused")
        || diagnostic.contains("unable to connect")
        || diagnostic.contains("no route to host")
    {
        io::ErrorKind::ConnectionRefused
    } else if diagnostic.contains("timed out") || diagnostic.contains("timeout") {
        io::ErrorKind::TimedOut
    } else if diagnostic.contains("cannot assign requested address")
        || diagnostic.contains("address not available")
    {
        io::ErrorKind::AddrNotAvailable
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(kind, format!("iperf3 exited with status {status}"))
}

fn parse_iperf3_json(output: &[u8]) -> io::Result<DirectionMeasurement> {
    let document: Value = serde_json::from_slice(output)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "iperf3 returned invalid JSON"))?;
    if let Some(error) = document.get("error").and_then(Value::as_str) {
        return Err(classify_iperf3_message(error));
    }

    let summary = document
        .pointer("/end/sum_received")
        .or_else(|| document.pointer("/end/sum"))
        .or_else(|| document.pointer("/end/sum_sent"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "iperf3 JSON did not contain a transfer summary",
            )
        })?;
    let bytes = summary
        .get("bytes")
        .and_then(json_u64)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid iperf3 byte count"))?;
    let bits_per_second = summary
        .get("bits_per_second")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.round().min(u64::MAX as f64) as u64)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid iperf3 throughput value",
            )
        })?;

    Ok(DirectionMeasurement {
        bits_per_second,
        bytes,
    })
}

fn json_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0 && *value <= u64::MAX as f64)
            .map(|value| value.round() as u64)
    })
}

fn classify_iperf3_message(message: &str) -> io::Error {
    let message = message.to_ascii_lowercase();
    let kind = if message.contains("server is busy") {
        io::ErrorKind::WouldBlock
    } else if message.contains("connection refused")
        || message.contains("unable to connect")
        || message.contains("no route to host")
    {
        io::ErrorKind::ConnectionRefused
    } else if message.contains("timed out") || message.contains("timeout") {
        io::ErrorKind::TimedOut
    } else if message.contains("cannot assign requested address")
        || message.contains("address not available")
    {
        io::ErrorKind::AddrNotAvailable
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(kind, "iperf3 reported a test failure")
}

/// Keeps a bundled iperf3 server aligned with the active EasyTier virtual IP.
/// A failed bind or an unexpected child exit is retried without terminating
/// the Windows service or disturbing Core's auto-connect supervision.
pub fn serve_iperf3_until(
    executable: &Path,
    stop: &AtomicBool,
    desired_target: &Mutex<Option<Iperf3BindTarget>>,
) -> io::Result<()> {
    if !executable.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "bundled iperf3 executable is unavailable",
        ));
    }

    let mut server: Option<Iperf3Server> = None;
    let mut configured_target = None;
    let mut retry_at = Instant::now();

    while !stop.load(Ordering::Acquire) {
        let target = *desired_target
            .lock()
            .map_err(|_| io::Error::other("iperf3 server target lock was poisoned"))?;

        if target != configured_target {
            server = None;
            configured_target = target;
            retry_at = Instant::now();
        }

        if let Some(active_server) = server.as_mut() {
            if active_server.try_wait()?.is_some() {
                server = None;
                retry_at = Instant::now() + SERVER_RESTART_DELAY;
            }
        }

        if server.is_none() && Instant::now() >= retry_at {
            if let Some(target) = target {
                match Iperf3Server::start(executable, target.address) {
                    Ok(started) => {
                        server = Some(started);
                    }
                    Err(_) => retry_at = Instant::now() + SERVER_RESTART_DELAY,
                }
            }
        }

        thread::sleep(Duration::from_millis(100));
    }

    drop(server);
    Ok(())
}

struct Iperf3Server {
    child: Child,
    #[cfg(windows)]
    _job: ProcessJob,
}

impl Iperf3Server {
    fn start(executable: &Path, address: Ipv4Addr) -> io::Result<Self> {
        let mut command = Command::new(executable);
        command
            .args([
                "--server",
                "--bind",
                &address.to_string(),
                "--port",
                &BANDWIDTH_TEST_PORT.to_string(),
                "--interval",
                "0",
                "--server-max-duration",
                &SERVER_MAX_DURATION_SECONDS.to_string(),
                "--version4",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_window(&mut command);
        #[cfg(windows)]
        let job = ProcessJob::new()?;
        let child = command.spawn()?;
        #[cfg(windows)]
        let child = {
            use std::os::windows::io::AsRawHandle;
            let mut child = child;
            if let Err(error) = job.assign(child.as_raw_handle() as isize) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            child
        };
        Ok(Self {
            child,
            #[cfg(windows)]
            _job: job,
        })
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
}

/// The iperf3 server belongs to the Windows service process tree. If SCM or
/// Windows terminates the service abruptly, kill-on-close prevents a stale
/// listener from surviving and blocking the recovered service instance.
#[cfg(windows)]
struct ProcessJob {
    handle: isize,
}

#[cfg(windows)]
impl ProcessJob {
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
            let error = io::Error::last_os_error();
            unsafe {
                CloseHandle(handle);
            }
            return Err(error);
        }
        Ok(Self { handle })
    }

    fn assign(&self, process_handle: isize) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.handle, process_handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
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
    fn CloseHandle(handle: isize) -> i32;
}

impl Drop for Iperf3Server {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn validate_duration(duration_seconds: u8) -> io::Result<()> {
    if (MIN_TEST_DURATION_SECONDS..=MAX_TEST_DURATION_SECONDS).contains(&duration_seconds) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "iperf3 test duration is out of range",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::sync::Arc;

    #[test]
    fn bind_target_extracts_a_valid_virtual_ipv4() {
        assert_eq!(
            Iperf3BindTarget::from_cidr("100.76.1.2/32"),
            Some(Iperf3BindTarget {
                address: "100.76.1.2".parse().unwrap()
            })
        );
        assert_eq!(Iperf3BindTarget::from_cidr("100.76.1.2/33"), None);
        assert_eq!(Iperf3BindTarget::from_cidr("not-an-address/24"), None);
    }

    #[test]
    fn client_arguments_bind_the_virtual_interface_and_request_json() {
        let arguments = iperf3_client_arguments(
            "100.76.1.2".parse().unwrap(),
            "100.76.1.9".parse().unwrap(),
            29_999,
            3,
            true,
        )
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--bind", "100.76.1.2"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--client", "100.76.1.9"]));
        assert!(arguments.windows(2).any(|pair| pair == ["--port", "29999"]));
        assert!(arguments.contains(&"--json".to_owned()));
        assert!(arguments.contains(&"--reverse".to_owned()));
    }

    #[test]
    fn parser_uses_the_received_summary_from_iperf3_json() {
        let output = br#"{
            "end": {
                "sum_sent": { "bytes": 8000, "bits_per_second": 64000.0 },
                "sum_received": { "bytes": 7500, "bits_per_second": 60000.4 }
            }
        }"#;
        assert_eq!(
            parse_iperf3_json(output).unwrap(),
            DirectionMeasurement {
                bits_per_second: 60_000,
                bytes: 7_500,
            }
        );
    }

    #[test]
    fn parser_rejects_missing_or_malformed_transfer_summaries() {
        assert_eq!(
            parse_iperf3_json(br#"{"end":{}}"#).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            parse_iperf3_json(b"not json").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn iperf3_busy_response_is_a_retryable_busy_error() {
        let error =
            parse_iperf3_json(br#"{"error":"the server is busy running a test"}"#).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn test_rejects_the_local_node_as_its_own_peer() {
        let address = Ipv4Addr::new(100, 76, 1, 2);
        let error = run_iperf3_test(Path::new("iperf3"), address, address, 3).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn executable_path_type_remains_owned_by_the_caller() {
        let path = std::path::PathBuf::from("C:\\Program Files\\Vibe EasyTier\\iperf3.exe");
        assert!(path.ends_with("iperf3.exe"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the staged Windows x64 iperf3 runtime"]
    fn bundled_iperf3_wrapper_measures_upload_and_reverse_download() {
        let executable = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("resources")
            .join("iperf3")
            .join("windows-x64")
            .join("iperf3.exe");
        assert!(
            executable.is_file(),
            "stage iperf3 before running this test"
        );

        let stop = Arc::new(AtomicBool::new(false));
        let target = Arc::new(Mutex::new(Some(Iperf3BindTarget {
            address: Ipv4Addr::new(127, 0, 0, 2),
        })));
        let server_stop = Arc::clone(&stop);
        let server_target = Arc::clone(&target);
        let server_executable = executable.clone();
        let server = thread::spawn(move || {
            serve_iperf3_until(&server_executable, &server_stop, &server_target)
        });
        thread::sleep(Duration::from_millis(500));

        let result = run_iperf3_test(
            &executable,
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::new(127, 0, 0, 2),
            1,
        );
        stop.store(true, Ordering::Release);
        server.join().unwrap().unwrap();

        let measurement = result.unwrap();
        assert!(measurement.upload_bps > 0);
        assert!(measurement.download_bps > 0);
        assert!(measurement.upload_bytes > 0);
        assert!(measurement.download_bytes > 0);
    }
}
