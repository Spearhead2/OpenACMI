//! Tacview real-time telemetry client.
//!
//! Protocol (Tacview "Real-Time Telemetry Public Protocol"): after the TCP
//! connection is established both sides send a NUL-terminated handshake.
//!
//! ```text
//! host:   XtraLib.Stream.0\nTacview.RealTimeTelemetry.0\n<host name>\n\0
//! client: XtraLib.Stream.0\nTacview.RealTimeTelemetry.0\n<client name>\n<password hash>\0
//! ```
//!
//! The host then streams plain ACMI 2.x text. A wrong password is not
//! reported: the host simply closes the socket.
//!
//! The client runs on its own thread, reconnects automatically and hands
//! complete lines to the application through a channel.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 42674;
const PROTOCOL_LOW: &str = "XtraLib.Stream.0";
const PROTOCOL_HIGH: &str = "Tacview.RealTimeTelemetry.0";
const RECONNECT_DELAY: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const CRC64_WE_POLY: u64 = 0x42f0_e1eb_a9ea_3693;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RttConfig {
    pub host: String,
    pub port: u16,
    pub client_name: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RttStatus {
    Connecting,
    Handshaking,
    Streaming {
        host_name: String,
    },
    /// Waiting to retry; the string says why the last attempt ended.
    Reconnecting(String),
    Stopped,
}

#[derive(Debug)]
pub enum RttMessage {
    /// A new stream begins: the receiver should start a fresh parser.
    StreamStarted,
    /// One or more complete lines of ACMI text.
    Lines(String),
}

/// Handle to the background client. Dropping it stops the thread.
pub struct RttClient {
    config: RttConfig,
    messages: Receiver<RttMessage>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<RttStatus>>,
    bytes_received: Arc<AtomicU64>,
}

impl RttClient {
    pub fn connect(config: RttConfig) -> Self {
        let (tx, messages) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(RttStatus::Connecting));
        let bytes_received = Arc::new(AtomicU64::new(0));
        let worker = Worker {
            config: config.clone(),
            tx,
            stop: Arc::clone(&stop),
            status: Arc::clone(&status),
            bytes_received: Arc::clone(&bytes_received),
        };
        std::thread::Builder::new()
            .name("acmi-rtt".into())
            .spawn(move || worker.run())
            .expect("failed to spawn telemetry thread");
        Self { config, messages, stop, status, bytes_received }
    }

    pub fn config(&self) -> &RttConfig {
        &self.config
    }

    pub fn status(&self) -> RttStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn bytes_received(&self) -> u64 {
        self.bytes_received.load(Ordering::Relaxed)
    }

    /// Next pending message, if any. Never blocks.
    pub fn try_recv(&self) -> Option<RttMessage> {
        self.messages.try_recv().ok()
    }
}

impl Drop for RttClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Worker {
    config: RttConfig,
    tx: Sender<RttMessage>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<RttStatus>>,
    bytes_received: Arc<AtomicU64>,
}

impl Worker {
    fn run(self) {
        while !self.stopped() {
            self.set_status(RttStatus::Connecting);
            let reason = match self.session() {
                Ok(()) => "connection closed by host".to_owned(),
                Err(err) => err.to_string(),
            };
            if self.stopped() {
                break;
            }
            log::warn!("telemetry {}:{}: {reason}", self.config.host, self.config.port);
            self.set_status(RttStatus::Reconnecting(reason));
            let retry_at = Instant::now() + RECONNECT_DELAY;
            while Instant::now() < retry_at && !self.stopped() {
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        self.set_status(RttStatus::Stopped);
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn set_status(&self, status: RttStatus) {
        *self.status.lock().unwrap() = status;
    }

    /// One connection, from connect to disconnect.
    fn session(&self) -> io::Result<()> {
        let address = (self.config.host.as_str(), self.config.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "host name did not resolve"))?;
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        stream.set_nodelay(true)?;
        // Short read timeouts keep the thread responsive to `stop`.
        stream.set_read_timeout(Some(POLL_INTERVAL))?;

        self.set_status(RttStatus::Handshaking);
        stream.write_all(client_handshake(&self.config).as_bytes())?;

        let mut buffer = Vec::with_capacity(64 * 1024);
        let mut chunk = vec![0u8; 64 * 1024];
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let host_name = loop {
            if let Some(nul) = buffer.iter().position(|&b| b == 0) {
                let host_name = parse_host_handshake(&buffer[..nul])?;
                buffer.drain(..=nul);
                break host_name;
            }
            if Instant::now() > deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "handshake timed out"));
            }
            if !self.read_some(&mut stream, &mut chunk, &mut buffer)? {
                return Ok(());
            }
        };

        log::info!("telemetry stream from {host_name:?} started");
        self.set_status(RttStatus::Streaming { host_name });
        if self.tx.send(RttMessage::StreamStarted).is_err() {
            return Ok(());
        }
        loop {
            // Forward whole lines only; a partial line waits for its remainder.
            if let Some(end) = buffer.iter().rposition(|&b| b == b'\n') {
                let lines = String::from_utf8_lossy(&buffer[..=end]).into_owned();
                buffer.drain(..=end);
                if self.tx.send(RttMessage::Lines(lines)).is_err() {
                    return Ok(());
                }
            }
            if !self.read_some(&mut stream, &mut chunk, &mut buffer)? {
                return Ok(());
            }
        }
    }

    /// Appends available bytes to `buffer`. Returns `false` when the session
    /// should end (peer closed the socket or a stop was requested).
    fn read_some(&self, stream: &mut TcpStream, chunk: &mut [u8], buffer: &mut Vec<u8>) -> io::Result<bool> {
        if self.stopped() {
            return Ok(false);
        }
        match stream.read(chunk) {
            Ok(0) => Ok(false),
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                self.bytes_received.fetch_add(n as u64, Ordering::Relaxed);
                Ok(true)
            }
            Err(err)
                if matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) =>
            {
                Ok(true)
            }
            Err(err) => Err(err),
        }
    }
}

fn client_handshake(config: &RttConfig) -> String {
    let hash = if config.password.is_empty() { "0".to_owned() } else { format!("{:x}", password_hash(&config.password)) };
    format!("{PROTOCOL_LOW}\n{PROTOCOL_HIGH}\n{}\n{hash}\0", config.client_name)
}

/// Validates the host handshake and returns the host's name.
fn parse_host_handshake(packet: &[u8]) -> io::Result<String> {
    let text = String::from_utf8_lossy(packet);
    let mut lines = text.lines().map(str::trim);
    if lines.next() != Some(PROTOCOL_LOW) || lines.next() != Some(PROTOCOL_HIGH) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "host does not speak the Tacview telemetry protocol"));
    }
    Ok(lines.next().unwrap_or("").to_owned())
}

/// CRC-64/WE of the password encoded as UTF-16LE, as the protocol requires.
pub fn password_hash(password: &str) -> u64 {
    crc64_we(password.encode_utf16().flat_map(u16::to_le_bytes))
}

fn crc64_we(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut crc = u64::MAX;
    for byte in bytes {
        crc ^= (byte as u64) << 56;
        for _ in 0..8 {
            crc = if crc & (1 << 63) != 0 { (crc << 1) ^ CRC64_WE_POLY } else { crc << 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn crc64_we_check_value() {
        // Catalogue check value for CRC-64/WE.
        assert_eq!(crc64_we(*b"123456789"), 0x62ec_59e3_f1a4_f00a);
    }

    #[test]
    fn password_hash_matches_published_vector() {
        // Test vector from the Tacview wiki: L"test".
        assert_eq!(password_hash("test"), 0x38e0_95a0_27ba_f2bc);
    }

    #[test]
    fn handshake_format() {
        let config = RttConfig { host: "h".into(), port: 1, client_name: "Me".into(), password: String::new() };
        assert_eq!(client_handshake(&config), "XtraLib.Stream.0\nTacview.RealTimeTelemetry.0\nMe\n0\0");
        assert_eq!(parse_host_handshake(b"XtraLib.Stream.0\nTacview.RealTimeTelemetry.0\nDCS\n").unwrap(), "DCS");
        assert!(parse_host_handshake(b"HTTP/1.1 200 OK\n").is_err());
    }

    #[test]
    fn streams_lines_from_a_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let host = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .write_all(b"XtraLib.Stream.0\nTacview.RealTimeTelemetry.0\nTestHost\n\0FileType=text/acmi/tacview\nFileVer")
                .unwrap();
            let mut reply = Vec::new();
            let mut byte = [0u8; 1];
            while socket.read(&mut byte).unwrap() == 1 && byte[0] != 0 {
                reply.push(byte[0]);
            }
            socket.write_all(b"sion=2.2\n#0\n1,T=1|2|3\npartial").unwrap();
            String::from_utf8(reply).unwrap()
        });

        let client = RttClient::connect(RttConfig {
            host: "127.0.0.1".into(),
            port,
            client_name: "Tester".into(),
            password: String::new(),
        });
        let mut text = String::new();
        let mut started = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !text.contains("1,T=1|2|3\n") && Instant::now() < deadline {
            match client.try_recv() {
                Some(RttMessage::StreamStarted) => started = true,
                Some(RttMessage::Lines(lines)) => text.push_str(&lines),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(started);
        assert_eq!(text, "FileType=text/acmi/tacview\nFileVersion=2.2\n#0\n1,T=1|2|3\n");
        assert!(host.join().unwrap().starts_with("XtraLib.Stream.0\nTacview.RealTimeTelemetry.0\nTester\n"));
    }
}
