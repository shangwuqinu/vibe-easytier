//! Length-prefixed JSON framing and a Windows named-pipe client.
//!
//! The framing layer is platform independent so a service host can apply its
//! own Windows pipe ACL and still use the same request handler in tests.

use std::{
    io::{self, Read, Write},
    time::Duration,
};

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

use crate::protocol::{RpcRequest, RpcResponse, PROTOCOL_VERSION};

pub const DEFAULT_PIPE_ENDPOINT: &str = r"\\.\pipe\VibeEasyTierService";
pub const WINDOWS_SERVICE_NAME: &str = "VibeEasyTierService";
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Accepts the canonical textual form produced by Windows for a user SID.
pub fn is_valid_owner_sid(value: &str) -> bool {
    let mut pieces = value.split('-');
    matches!(pieces.next(), Some("S"))
        && matches!(pieces.next(), Some("1"))
        && pieces.clone().count() >= 2
        && value.len() <= 184
        && pieces.all(|piece| !piece.is_empty() && piece.parse::<u64>().is_ok())
}

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("the Vibe EasyTier service is unavailable: {0}")]
    Unavailable(String),
    #[error("IPC I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("IPC serialization failed: {0}")]
    Serialization(String),
    #[error("IPC frame is too large: {size} bytes (limit {limit})")]
    FrameTooLarge { size: usize, limit: usize },
    #[error("IPC protocol mismatch: {0}")]
    Protocol(String),
}

pub trait RpcHandler {
    fn handle_rpc(&mut self, request: RpcRequest) -> RpcResponse;
}

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, message: &T) -> Result<(), IpcError> {
    let payload =
        serde_json::to_vec(message).map_err(|error| IpcError::Serialization(error.to_string()))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge {
            size: payload.len(),
            limit: MAX_FRAME_BYTES,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| IpcError::FrameTooLarge {
        size: payload.len(),
        limit: MAX_FRAME_BYTES,
    })?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, IpcError> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let size = u32::from_le_bytes(length) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge {
            size,
            limit: MAX_FRAME_BYTES,
        });
    }
    let mut payload = vec![0_u8; size];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload).map_err(|error| IpcError::Serialization(error.to_string()))
}

/// Handles exactly one request on a connected byte stream.
pub fn serve_one<S: Read + Write, H: RpcHandler>(
    stream: &mut S,
    handler: &mut H,
) -> Result<(), IpcError> {
    let request: RpcRequest = read_frame(stream)?;
    let response = handler.handle_rpc(request);
    write_frame(stream, &response)
}

#[derive(Clone, Debug)]
pub struct IpcClient {
    endpoint: String,
    connect_timeout: Duration,
}

/// Backwards-compatible short name for desktop code.
pub type Client = IpcClient;

impl Default for IpcClient {
    fn default() -> Self {
        Self::windows_default()
    }
}

impl IpcClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            connect_timeout: Duration::from_secs(3),
        }
    }

    pub fn windows_default() -> Self {
        Self::new(DEFAULT_PIPE_ENDPOINT)
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn call(&self, request: &RpcRequest) -> Result<RpcResponse, IpcError> {
        if !request.is_compatible() {
            return Err(IpcError::Protocol(format!(
                "request protocol version {} is unsupported (expected {PROTOCOL_VERSION})",
                request.protocol_version
            )));
        }

        #[cfg(windows)]
        {
            self.call_windows(request)
        }

        #[cfg(not(windows))]
        {
            let _ = request;
            Err(IpcError::Unavailable(
                "Windows named pipes are unavailable on this platform".to_owned(),
            ))
        }
    }

    #[cfg(windows)]
    fn call_windows(&self, request: &RpcRequest) -> Result<RpcResponse, IpcError> {
        use std::{fs::OpenOptions, thread, time::Instant};

        let started_at = Instant::now();
        loop {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.endpoint)
            {
                Ok(mut pipe) => {
                    if self.endpoint == DEFAULT_PIPE_ENDPOINT {
                        verify_pipe_server_identity(&pipe).map_err(|error| {
                            IpcError::Unavailable(format!(
                                "refusing to send a request to an untrusted pipe server at {}: {error}",
                                self.endpoint
                            ))
                        })?;
                    }
                    write_frame(&mut pipe, request)?;
                    let response: RpcResponse = read_frame(&mut pipe)?;
                    if response.protocol_version != PROTOCOL_VERSION {
                        return Err(IpcError::Protocol(format!(
                            "service protocol version {} is unsupported",
                            response.protocol_version
                        )));
                    }
                    if response.request_id != request.request_id {
                        return Err(IpcError::Protocol(
                            "service response request_id does not match the request".to_owned(),
                        ));
                    }
                    return Ok(response);
                }
                Err(error)
                    if should_retry_pipe_open(&error)
                        && started_at.elapsed() < self.connect_timeout =>
                {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    return Err(IpcError::Unavailable(format!(
                        "could not open {} after {} seconds: {}",
                        self.endpoint,
                        started_at.elapsed().as_secs(),
                        error
                    )));
                }
            }
        }
    }
}

#[cfg(windows)]
fn should_retry_pipe_open(error: &io::Error) -> bool {
    // ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, and ERROR_PIPE_NOT_CONNECTED.
    matches!(error.raw_os_error(), Some(2 | 231 | 233))
}

/// Verifies the predictable production pipe only after it is connected. The
/// service-owned DACL is the primary boundary; this closes the small startup
/// race where another local process could otherwise pre-create the fixed name.
/// Test-only custom endpoints intentionally skip this check so they can use an
/// isolated in-process pipe rather than the SCM-registered service.
#[cfg(windows)]
fn verify_pipe_server_identity(pipe: &std::fs::File) -> io::Result<()> {
    let server_process_id = pipe_server_process_id(pipe)?;
    let service_process_id = registered_service_process_id()?;
    if service_process_id != server_process_id {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "named-pipe server is not the registered Vibe EasyTier service process",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn pipe_server_process_id(pipe: &std::fs::File) -> io::Result<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;

    let mut server_process_id = 0_u32;
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut server_process_id) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if server_process_id == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "named-pipe server did not provide a process id",
        ));
    }
    Ok(server_process_id)
}

/// Uses the SCM status API rather than opening a LocalSystem process token.
/// A normal desktop account is allowed to query its service status but can be
/// denied `OpenProcessToken(TOKEN_QUERY)` for the LocalSystem process itself.
#[cfg(windows)]
fn registered_service_process_id() -> io::Result<u32> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, SC_MANAGER_CONNECT,
        SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS_PROCESS,
    };

    let manager = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT) };
    if manager.is_null() {
        return Err(io::Error::last_os_error());
    }
    let service_name: Vec<u16> = OsStr::new(WINDOWS_SERVICE_NAME)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = (|| {
        let service = unsafe { OpenServiceW(manager, service_name.as_ptr(), SERVICE_QUERY_STATUS) };
        if service.is_null() {
            return Err(io::Error::last_os_error());
        }
        let status_result = (|| {
            let mut status = std::mem::MaybeUninit::<SERVICE_STATUS_PROCESS>::zeroed();
            let mut bytes_needed = 0_u32;
            if unsafe {
                QueryServiceStatusEx(
                    service,
                    SC_STATUS_PROCESS_INFO,
                    status.as_mut_ptr().cast::<u8>(),
                    std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
                    &mut bytes_needed,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let status = unsafe { status.assume_init() };
            if status.dwCurrentState != SERVICE_RUNNING || status.dwProcessId == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "Vibe EasyTier service is not running",
                ));
            }
            Ok(status.dwProcessId)
        })();
        unsafe {
            CloseServiceHandle(service);
        }
        status_result
    })();
    unsafe {
        CloseServiceHandle(manager);
    }
    result
}

/// Runs a synchronous Windows named-pipe endpoint. The pipe ACL grants the
/// configured owner SID read/write access, plus full access to LocalSystem and
/// administrators; remote pipe clients are rejected by the pipe itself.
///
/// Each connection carries exactly one framed request. Run this on a dedicated
/// thread because `ConnectNamedPipe` is intentionally blocking. A service
/// process can simply drop the thread during shutdown after setting `stop`.
#[cfg(windows)]
pub fn serve_windows_pipe_until<H: RpcHandler>(
    endpoint: &str,
    owner_sid: Option<&str>,
    stop: &std::sync::atomic::AtomicBool,
    handler: &std::sync::Mutex<H>,
) -> Result<(), IpcError> {
    use std::sync::atomic::Ordering;

    let server = WindowsPipeServer::new(endpoint, owner_sid)?;
    while !stop.load(Ordering::Acquire) {
        let mut connection = match server.accept() {
            Ok(connection) => connection,
            Err(_) if !stop.load(Ordering::Acquire) => {
                // A transient pipe-creation failure must not make the IPC
                // host permanently disappear. Access is enforced by the
                // pipe DACL created by this LocalSystem service.
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        // Do not retain the service-state lock while waiting for untrusted IPC
        // input. A stalled desktop client must not pause core supervision.
        let request = read_frame(&mut connection);
        if let Ok(request) = request {
            let response = handler
                .lock()
                .map_err(|_| {
                    IpcError::Unavailable("service RPC handler lock was poisoned".to_owned())
                })?
                .handle_rpc(request);
            let _ = write_frame(&mut connection, &response);
        }
    }
    Ok(())
}

#[cfg(windows)]
struct WindowsPipeServer {
    endpoint: Vec<u16>,
    owner_sid: Option<String>,
}

#[cfg(windows)]
impl WindowsPipeServer {
    fn new(endpoint: &str, owner_sid: Option<&str>) -> Result<Self, IpcError> {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

        if !endpoint.starts_with(r"\\.\pipe\") {
            return Err(IpcError::Protocol(
                "Windows IPC endpoint must be a local named-pipe path".to_owned(),
            ));
        }
        if owner_sid.is_some_and(|sid| !is_valid_owner_sid(sid)) {
            return Err(IpcError::Protocol(
                "configured pipe owner SID is invalid".to_owned(),
            ));
        }
        let endpoint = OsStr::new(endpoint).encode_wide().chain(Some(0)).collect();
        Ok(Self {
            endpoint,
            owner_sid: owner_sid.map(ToOwned::to_owned),
        })
    }

    fn accept(&self) -> io::Result<WindowsPipeConnection> {
        let security = PipeSecurity::for_owner(self.owner_sid.as_deref())?;
        let handle = unsafe {
            CreateNamedPipeW(
                self.endpoint.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_FRAME_BYTES as u32,
                MAX_FRAME_BYTES as u32,
                0,
                security.attributes_ptr(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        if connected == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED) {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(error);
            }
        }
        Ok(WindowsPipeConnection { handle })
    }
}

#[cfg(windows)]
struct WindowsPipeConnection {
    handle: isize,
}

#[cfg(windows)]
impl Read for WindowsPipeConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut read = 0_u32;
        let byte_count = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let success = unsafe {
            ReadFile(
                self.handle,
                buffer.as_mut_ptr().cast(),
                byte_count,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(read as usize)
    }
}

#[cfg(windows)]
impl Write for WindowsPipeConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut written = 0_u32;
        let byte_count = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let success = unsafe {
            WriteFile(
                self.handle,
                buffer.as_ptr().cast(),
                byte_count,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        // DisconnectNamedPipe can discard a just-written response before the
        // client has a chance to read it. Flushing the server end provides the
        // required request/response handoff before this connection is dropped.
        let success = unsafe { FlushFileBuffers(self.handle) };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for WindowsPipeConnection {
    fn drop(&mut self) {
        unsafe {
            DisconnectNamedPipe(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut std::ffi::c_void,
    inherit_handle: i32,
}

#[cfg(windows)]
struct PipeSecurity {
    descriptor: *mut std::ffi::c_void,
    attributes: SecurityAttributes,
}

#[cfg(windows)]
impl PipeSecurity {
    fn for_owner(owner_sid: Option<&str>) -> io::Result<Self> {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

        // No broad Interactive Users ACE: a second local account must not be
        // able to reconfigure someone else's private network.
        let sddl = match owner_sid {
            Some(owner_sid) => format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{owner_sid})"),
            None => "D:P(A;;GA;;;SY)(A;;GA;;;BA)".to_owned(),
        };
        let sddl: Vec<u16> = OsStr::new(&sddl).encode_wide().chain(Some(0)).collect();
        let mut descriptor = std::ptr::null_mut();
        let created = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if created == 0 {
            return Err(io::Error::last_os_error());
        }
        let attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            security_descriptor: descriptor,
            inherit_handle: 0,
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }

    fn attributes_ptr(&self) -> *mut SecurityAttributes {
        &self.attributes as *const SecurityAttributes as *mut SecurityAttributes
    }
}

#[cfg(windows)]
impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

#[cfg(windows)]
const INVALID_HANDLE_VALUE: isize = -1;
#[cfg(windows)]
const ERROR_PIPE_CONNECTED: i32 = 535;
#[cfg(windows)]
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
#[cfg(windows)]
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
#[cfg(windows)]
const SDDL_REVISION_1: u32 = 1;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        maximum_instances: u32,
        output_buffer_size: u32,
        input_buffer_size: u32,
        default_timeout: u32,
        security_attributes: *mut SecurityAttributes,
    ) -> isize;
    fn ConnectNamedPipe(handle: isize, overlapped: *mut std::ffi::c_void) -> i32;
    fn DisconnectNamedPipe(handle: isize) -> i32;
    fn ReadFile(
        handle: isize,
        buffer: *mut std::ffi::c_void,
        bytes_to_read: u32,
        bytes_read: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn WriteFile(
        handle: isize,
        buffer: *const std::ffi::c_void,
        bytes_to_write: u32,
        bytes_written: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn FlushFileBuffers(handle: isize) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

#[cfg(windows)]
#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string_security_descriptor: *const u16,
        revision: u32,
        security_descriptor: *mut *mut std::ffi::c_void,
        security_descriptor_size: *mut u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::{
        profile::{AddressMode, EasyTierFlags, NetworkProfile, SecretString},
        protocol::{ProfileUpsert, RpcCommand, RpcRequest, RpcResult},
    };

    use super::*;

    #[test]
    fn length_prefixed_requests_round_trip() {
        let request = RpcRequest::new(9, RpcCommand::Ping);
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request).unwrap();

        let decoded: RpcRequest = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn frame_limit_is_enforced_before_allocating() {
        let mut bytes = ((MAX_FRAME_BYTES + 1) as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(b"ignored");

        assert!(matches!(
            read_frame::<_, RpcRequest>(&mut Cursor::new(bytes)),
            Err(IpcError::FrameTooLarge { .. })
        ));
    }

    struct Echo;

    impl RpcHandler for Echo {
        fn handle_rpc(&mut self, request: RpcRequest) -> RpcResponse {
            RpcResponse::ok(request.request_id, RpcResult::Pong)
        }
    }

    #[test]
    fn server_helper_handles_a_single_request() {
        let mut input = Vec::new();
        write_frame(&mut input, &RpcRequest::new(4, RpcCommand::Ping)).unwrap();
        let request_len = input.len();
        let mut stream = Cursor::new(input);
        let mut handler = Echo;

        serve_one(&mut stream, &mut handler).unwrap();
        let response: RpcResponse = read_frame(&mut Cursor::new(
            stream.into_inner()[request_len..].to_vec(),
        ))
        .unwrap();
        assert_eq!(response.result, Some(RpcResult::Pong));
    }

    #[test]
    fn owner_sid_validation_rejects_non_sid_strings() {
        assert!(is_valid_owner_sid("S-1-5-21-123-456-789-1001"));
        assert!(!is_valid_owner_sid("interactive-users"));
        assert!(!is_valid_owner_sid("S-1-5"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_client_round_trips_through_owner_acl_pipe() {
        use std::{process::Command, thread};

        let output = Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .expect("whoami should be available on Windows");
        assert!(output.status.success());
        let owner_sid = String::from_utf8(output.stdout)
            .expect("whoami output should be UTF-8")
            .split(',')
            .last()
            .map(|value| value.trim().trim_matches('"').to_owned())
            .filter(|value| is_valid_owner_sid(value))
            .expect("whoami should return the current user SID");
        let endpoint = format!(
            r"\\.\pipe\VibeEasyTierIpcTest{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        );
        let endpoint_for_server = endpoint.clone();
        let server = thread::spawn(move || {
            let server = WindowsPipeServer::new(&endpoint_for_server, Some(&owner_sid)).unwrap();
            let mut connection = server.accept().unwrap();
            serve_one(&mut connection, &mut Echo).unwrap();
        });

        thread::sleep(Duration::from_millis(50));
        let response = IpcClient::new(endpoint)
            .with_connect_timeout(Duration::from_secs(2))
            .call(&RpcRequest::new(92, RpcCommand::Ping))
            .unwrap();
        assert_eq!(response.result, Some(RpcResult::Pong));
        server.join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a registered local VibeEasyTierService"]
    fn installed_service_answers_read_only_requests() {
        let client = IpcClient::windows_default();
        let ping = client
            .call(&RpcRequest::new(93, RpcCommand::Ping))
            .expect("the registered service should respond to Ping");
        assert_eq!(ping.result, Some(RpcResult::Pong));

        let status = client
            .call(&RpcRequest::new(94, RpcCommand::GetStatus))
            .expect("the registered service should answer GetStatus");
        assert!(matches!(status.result, Some(RpcResult::Status(_))));

        let profiles = client
            .call(&RpcRequest::new(95, RpcCommand::ListProfiles))
            .expect("the registered service should answer ListProfiles");
        assert!(matches!(profiles.result, Some(RpcResult::Profiles(_))));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "creates and removes a temporary profile in the registered local service"]
    fn installed_service_persists_a_profile_round_trip() {
        let profile_id = format!("ipc-persistence-{}", std::process::id());
        let profile = NetworkProfile {
            id: profile_id.clone(),
            name: "IPC persistence check".to_owned(),
            instance_name: format!("vibe-{profile_id}"),
            hostname: "VibeEasyTierIpcTest".to_owned(),
            network_name: "vibe-ipc-check".to_owned(),
            network_secret: SecretString::new("live-ipc-test-secret"),
            address_mode: AddressMode::Static {
                cidr: "10.253.253.2/24".to_owned(),
            },
            // Exercise the installed service's profile path with the
            // EasyTier WireGuard peer transport, without connecting it.
            peers: vec!["wg://127.0.0.1:11012".to_owned()],
            flags: EasyTierFlags::default(),
            auto_connect: false,
        };
        let client = IpcClient::windows_default();

        let result = (|| {
            let saved = client.call(&RpcRequest::new(
                96,
                RpcCommand::UpsertProfile(ProfileUpsert {
                    profile,
                    make_active: false,
                }),
            ))?;
            assert!(matches!(saved.result, Some(RpcResult::ProfileSaved(_))));

            let mut flags = EasyTierFlags::default();
            flags.latency_first = true;
            flags.data_compress_algo = 2;
            flags.foreign_relay_bps_limit = 123_456;
            let updated = client.call(&RpcRequest::new(
                97,
                RpcCommand::UpdateProfileFlags {
                    profile_id: profile_id.clone(),
                    flags: flags.clone(),
                },
            ))?;
            let Some(RpcResult::ProfileSaved(updated)) = updated.result else {
                panic!("the service returned an invalid flag-update response");
            };
            assert_eq!(updated.flags, flags);
            assert!(updated.secret_configured);
            assert!(!format!("{updated:?}").contains("live-ipc-test-secret"));

            let listed = client.call(&RpcRequest::new(98, RpcCommand::ListProfiles))?;
            let Some(RpcResult::Profiles(profiles)) = listed.result else {
                panic!("the service returned an invalid profile-list response");
            };
            assert!(profiles.iter().any(|profile| profile.id == profile_id));
            Ok::<(), IpcError>(())
        })();

        let deleted = client.call(&RpcRequest::new(
            99,
            RpcCommand::DeleteProfile {
                profile_id: profile_id.clone(),
            },
        ));
        assert!(matches!(
            deleted.as_ref().map(|response| &response.result),
            Ok(Some(RpcResult::ProfileDeleted { profile_id: deleted_id })) if deleted_id == &profile_id
        ));
        result.expect("the registered service should durably save and list the temporary profile");
    }
}
