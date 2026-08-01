//! Bounded bandwidth probes over the EasyTier virtual network.
//!
//! The listener binds only to the active profile's virtual IPv4 address. It
//! deliberately does not use the Core management RPC and accepts requests
//! only from peers currently reported by EasyTier Core.

use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const BANDWIDTH_TEST_PORT: u16 = 29_999;
pub const DEFAULT_TEST_DURATION_SECONDS: u8 = 3;
const MIN_TEST_DURATION_SECONDS: u8 = 1;
const MAX_TEST_DURATION_SECONDS: u8 = 10;
const PROTOCOL_MAGIC: &[u8; 8] = b"VETBW001";
const REQUEST_BYTES: usize = 10;
const MODE_DOWNLOAD: u8 = 1;
const MODE_UPLOAD: u8 = 2;
const ACK: u8 = 0xa5;
const PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_CONCURRENT_TESTS: usize = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BandwidthBindTarget {
    pub address: Ipv4Addr,
    allowed_peers: Vec<Ipv4Addr>,
}

impl BandwidthBindTarget {
    pub fn from_cidr(cidr: &str) -> Option<Self> {
        let (address, prefix_length) = cidr.trim().split_once('/')?;
        let address = address.parse().ok()?;
        let prefix_length = prefix_length.parse::<u8>().ok()?;
        (prefix_length <= 32).then_some(Self {
            address,
            allowed_peers: Vec::new(),
        })
    }

    pub fn with_allowed_peers(mut self, peers: impl IntoIterator<Item = Ipv4Addr>) -> Self {
        self.allowed_peers.extend(peers);
        self.allowed_peers.sort_unstable();
        self.allowed_peers.dedup();
        self
    }

    fn allows(&self, address: Ipv4Addr) -> bool {
        address != self.address && self.allowed_peers.binary_search(&address).is_ok()
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

/// Runs one receive test followed by one send test. Sequential directions
/// avoid measuring two competing streams and make the result easier to read.
pub fn run_bandwidth_test(
    peer_address: Ipv4Addr,
    duration_seconds: u8,
) -> io::Result<BandwidthMeasurement> {
    run_bandwidth_test_to(
        SocketAddr::new(IpAddr::V4(peer_address), BANDWIDTH_TEST_PORT),
        duration_seconds,
    )
}

fn run_bandwidth_test_to(
    address: SocketAddr,
    duration_seconds: u8,
) -> io::Result<BandwidthMeasurement> {
    validate_duration(duration_seconds)?;
    let (download_bytes, download_elapsed) = measure_download(address, duration_seconds)?;
    let (upload_bytes, upload_elapsed) = measure_upload(address, duration_seconds)?;

    Ok(BandwidthMeasurement {
        download_bps: bits_per_second(download_bytes, download_elapsed),
        upload_bps: bits_per_second(upload_bytes, upload_elapsed),
        download_bytes,
        upload_bytes,
        duration_seconds,
    })
}

/// Keeps a nonblocking listener aligned with the active EasyTier virtual IP.
/// Bind failures are retried because the TUN adapter can disappear briefly
/// during Core recovery and a transient conflict must not disable the feature.
pub fn serve_until(
    stop: &AtomicBool,
    desired_target: &Mutex<Option<BandwidthBindTarget>>,
) -> io::Result<()> {
    let active_tests = Arc::new(AtomicUsize::new(0));
    let mut active_target = None;
    let mut listener: Option<TcpListener> = None;

    while !stop.load(Ordering::Acquire) {
        let target = desired_target
            .lock()
            .map_err(|_| io::Error::other("bandwidth listener target lock was poisoned"))?
            .clone();

        if target != active_target {
            listener = None;
            active_target = None;
            if let Some(target) = target {
                match TcpListener::bind((target.address, BANDWIDTH_TEST_PORT)) {
                    Ok(new_listener) => {
                        new_listener.set_nonblocking(true)?;
                        listener = Some(new_listener);
                        active_target = Some(target);
                    }
                    Err(_) => {
                        // The virtual adapter may not have materialized yet,
                        // and a transient port conflict must not permanently
                        // disable tests until the Windows service restarts.
                        thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                }
            }
        }

        let (Some(listener), Some(target)) = (&listener, active_target.as_ref()) else {
            thread::sleep(Duration::from_millis(250));
            continue;
        };

        match listener.accept() {
            Ok((stream, remote)) if remote_is_allowed(target, remote) => {
                if active_tests.fetch_add(1, Ordering::AcqRel) >= MAX_CONCURRENT_TESTS {
                    active_tests.fetch_sub(1, Ordering::AcqRel);
                    drop(stream);
                    continue;
                }
                let session_tests = Arc::clone(&active_tests);
                if thread::Builder::new()
                    .name("vibe-easytier-bandwidth-session".to_owned())
                    .spawn(move || {
                        let _ = handle_connection(stream);
                        session_tests.fetch_sub(1, Ordering::AcqRel);
                    })
                    .is_err()
                {
                    active_tests.fetch_sub(1, Ordering::AcqRel);
                }
            }
            Ok((stream, _)) => drop(stream),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

fn remote_is_allowed(target: &BandwidthBindTarget, remote: SocketAddr) -> bool {
    matches!(remote.ip(), IpAddr::V4(address) if target.allows(address))
}

fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let (mode, duration_seconds) = read_request(&mut stream)?;
    stream.write_all(&[ACK])?;

    match mode {
        MODE_DOWNLOAD => serve_download(&mut stream, duration_seconds),
        MODE_UPLOAD => serve_upload(&mut stream, duration_seconds),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported bandwidth test mode",
        )),
    }
}

fn serve_download(stream: &mut TcpStream, duration_seconds: u8) -> io::Result<()> {
    let payload = payload_buffer();
    let deadline = Instant::now() + Duration::from_secs(u64::from(duration_seconds));
    while Instant::now() < deadline {
        stream.write_all(&payload)?;
    }
    stream.shutdown(Shutdown::Write)
}

fn serve_upload(stream: &mut TcpStream, duration_seconds: u8) -> io::Result<()> {
    let mut buffer = [0_u8; PAYLOAD_BYTES];
    let mut received = 0_u64;
    let deadline = Instant::now() + Duration::from_secs(u64::from(duration_seconds) + 3);
    while Instant::now() < deadline {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes) => received = received.saturating_add(bytes as u64),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(error) => return Err(error),
        }
    }
    stream.write_all(&received.to_le_bytes())
}

fn measure_download(address: SocketAddr, duration_seconds: u8) -> io::Result<(u64, Duration)> {
    let mut stream = connect(address, duration_seconds)?;
    write_request(&mut stream, MODE_DOWNLOAD, duration_seconds)?;
    read_ack(&mut stream)?;

    let started = Instant::now();
    let mut received = 0_u64;
    let mut buffer = [0_u8; PAYLOAD_BYTES];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes) => received = received.saturating_add(bytes as u64),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "bandwidth download timed out",
                ));
            }
            Err(error) => return Err(error),
        }
    }
    Ok((received, started.elapsed()))
}

fn measure_upload(address: SocketAddr, duration_seconds: u8) -> io::Result<(u64, Duration)> {
    let mut stream = connect(address, duration_seconds)?;
    write_request(&mut stream, MODE_UPLOAD, duration_seconds)?;
    read_ack(&mut stream)?;

    let payload = payload_buffer();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(u64::from(duration_seconds));
    while Instant::now() < deadline {
        stream.write_all(&payload)?;
    }
    let elapsed = started.elapsed();
    stream.shutdown(Shutdown::Write)?;

    let mut received = [0_u8; 8];
    stream.read_exact(&mut received)?;
    Ok((u64::from_le_bytes(received), elapsed))
}

fn connect(address: SocketAddr, duration_seconds: u8) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(4))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(u64::from(duration_seconds) + 5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    Ok(stream)
}

fn write_request(stream: &mut TcpStream, mode: u8, duration_seconds: u8) -> io::Result<()> {
    validate_duration(duration_seconds)?;
    let mut request = [0_u8; REQUEST_BYTES];
    request[..PROTOCOL_MAGIC.len()].copy_from_slice(PROTOCOL_MAGIC);
    request[8] = mode;
    request[9] = duration_seconds;
    stream.write_all(&request)
}

fn read_request(stream: &mut TcpStream) -> io::Result<(u8, u8)> {
    let mut request = [0_u8; REQUEST_BYTES];
    stream.read_exact(&mut request)?;
    if &request[..PROTOCOL_MAGIC.len()] != PROTOCOL_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid bandwidth test protocol",
        ));
    }
    validate_duration(request[9])?;
    Ok((request[8], request[9]))
}

fn read_ack(stream: &mut TcpStream) -> io::Result<()> {
    let mut ack = [0_u8; 1];
    stream.read_exact(&mut ack)?;
    if ack[0] != ACK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "remote bandwidth service rejected the request",
        ));
    }
    Ok(())
}

fn validate_duration(duration_seconds: u8) -> io::Result<()> {
    if (MIN_TEST_DURATION_SECONDS..=MAX_TEST_DURATION_SECONDS).contains(&duration_seconds) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bandwidth test duration is out of range",
        ))
    }
}

fn payload_buffer() -> Vec<u8> {
    let mut state = 0x4d59_5df4_d0f3_3173_u64;
    let mut payload = vec![0_u8; PAYLOAD_BYTES];
    for chunk in payload.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes()[..chunk.len()]);
    }
    payload
}

fn bits_per_second(bytes: u64, elapsed: Duration) -> u64 {
    let millis = elapsed.as_millis().max(1);
    ((u128::from(bytes) * 8 * 1_000) / millis).min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_target_accepts_only_current_easytier_peers() {
        let target = BandwidthBindTarget::from_cidr("100.76.1.2/32")
            .unwrap()
            .with_allowed_peers(["100.76.1.9".parse().unwrap(), "100.76.2.9".parse().unwrap()]);
        assert!(target.allows("100.76.1.9".parse().unwrap()));
        assert!(target.allows("100.76.2.9".parse().unwrap()));
        assert!(!target.allows("100.76.1.10".parse().unwrap()));
        assert!(!target.allows("100.76.1.2".parse().unwrap()));
        assert_eq!(BandwidthBindTarget::from_cidr("100.76.1.2/33"), None);
    }

    #[test]
    fn loopback_probe_measures_both_directions() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                handle_connection(stream).unwrap();
            }
        });

        let result = run_bandwidth_test_to(address, 1).unwrap();
        server.join().unwrap();

        assert!(result.download_bytes > 0);
        assert!(result.upload_bytes > 0);
        assert!(result.download_bps > 0);
        assert!(result.upload_bps > 0);
    }
}
