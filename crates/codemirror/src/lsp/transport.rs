//! A native LSP subprocess connection on Unix.
//!
//! The pinned CodeMirror transport exchanges complete JSON messages. This
//! adapter adds LSP byte framing, runs the subprocess, and drives `LspSession`.
//! The host still owns the workspace, server request handlers, and editor UI.

#![cfg(unix)]

use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsString,
    fmt,
    io::{self, Read, Write},
    num::NonZeroUsize,
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Map, Value};

use super::rpc::{
    FrameError, LspEvent, LspSession, LspState, RequestId, ResponseError, RpcError, RpcId,
    RpcMessage, RpcResolution, StdioCodec,
};

mod client;

pub use client::{
    DeferredNotification, DeferredRequest, PreparedRequest, StdioClient, StdioClientError,
};

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_PENDING: NonZeroUsize = NonZeroUsize::new(64).unwrap();
const MAX_QUEUED: usize = 8;
const IO_CHUNK_BYTES: usize = 8 * 1024;
const IDLE_INTERVAL: Duration = Duration::from_millis(2);

/// An explicitly selected, trusted executable and its literal arguments.
///
/// Use an absolute path to a trusted executable. Arguments are not interpreted
/// by a shell. The child starts in the host's current directory with no inherited
/// environment. If a tool needs environment variables, use a trusted launcher
/// that sets only the values it needs. Do not put credentials in arguments.
pub struct Program {
    executable: PathBuf,
    args: Vec<OsString>,
}

impl Program {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }
}

/// A setup, send, connection, or shutdown error.
#[derive(Debug)]
pub enum TransportError {
    InvalidProgram,
    Io {
        operation: &'static str,
        error: io::Error,
    },
    Frame(FrameError),
    Rpc(RpcError),
    QueueFull,
    /// The status is present only if the process had already exited at EOF.
    Eof(Option<ExitStatus>),
    ShutdownTimedOut,
    ShutdownRejected(ResponseError),
    InvalidShutdownResult,
    UnsuccessfulExit(ExitStatus),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProgram => f.write_str("the LSP executable path must be absolute"),
            Self::Io { operation, error } => write!(f, "cannot {operation}: {error}"),
            Self::Frame(error) => error.fmt(f),
            Self::Rpc(error) => error.fmt(f),
            Self::QueueFull => f.write_str("the LSP output queue is full"),
            Self::Eof(Some(status)) => write!(f, "LSP stdout closed ({status})"),
            Self::Eof(None) => f.write_str("LSP stdout closed"),
            Self::ShutdownTimedOut => f.write_str("LSP shutdown timed out"),
            Self::ShutdownRejected(error) => write!(f, "LSP shutdown rejected ({})", error.code),
            Self::InvalidShutdownResult => f.write_str("the LSP shutdown result must be null"),
            Self::UnsuccessfulExit(status) => write!(f, "LSP process exited with {status}"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            Self::Frame(error) => Some(error),
            Self::Rpc(error) => Some(error),
            _ => None,
        }
    }
}

impl From<FrameError> for TransportError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<RpcError> for TransportError {
    fn from(error: RpcError) -> Self {
        Self::Rpc(error)
    }
}

/// An event from a single subprocess connection.
#[derive(Debug)]
pub enum TransportEvent {
    /// The `initialized` notification is queued before further client messages.
    Initialized,
    /// A request response, including a peer rejection of `initialize`.
    Resolution(RpcResolution),
    /// A server notification or request. Use `reply` for a server request.
    Message(RpcMessage),
    TimedOut(RequestId),
    /// A recoverable message error, or the error that failed initialization.
    ProtocolError(RpcError),
    Disconnected {
        reason: TransportError,
        pending: Vec<RequestId>,
    },
}

/// One LSP session over a child process's stdin and stdout.
///
/// Calls queue outgoing frames. Call [`Self::next_event`] to send frames and
/// receive events. There are no worker threads. The connection limits each
/// header to 8 KiB and each body to 8 MiB, with at most 64 pending requests and
/// 8 queued frames (plus one internal `initialized` notification). Stderr is
/// discarded. Use [`Self::shutdown`] for a graceful stop; dropping the transport
/// closes the streams and kills and reaps the direct child, not its descendants.
pub struct StdioTransport {
    child: Child,
    stdin: Option<UnixStream>,
    stdout: Option<UnixStream>,
    codec: StdioCodec,
    session: LspSession,
    deadlines: BTreeMap<RequestId, Deadline>,
    outgoing: VecDeque<Outbound>,
    incoming: VecDeque<Result<RpcMessage, RpcError>>,
    disconnect_reason: Option<TransportError>,
}

impl StdioTransport {
    pub fn spawn(program: &Program) -> Result<Self, TransportError> {
        if !program.executable.is_absolute() {
            return Err(TransportError::InvalidProgram);
        }
        let codec = StdioCodec::new(MAX_HEADER_BYTES, MAX_BODY_BYTES)?;
        // Only the parent endpoints are nonblocking. We can close them without
        // waiting for an I/O worker, even if a descendant holds a child endpoint.
        let (stdin, child_stdin) = stdio_pair()?;
        let (stdout, child_stdout) = stdio_pair()?;
        let child = Command::new(&program.executable)
            .args(&program.args)
            .env_clear()
            .stdin(child_stdin)
            .stdout(child_stdout)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| io_error("start LSP process", error))?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: Some(stdout),
            codec,
            session: LspSession::new(MAX_PENDING),
            deadlines: BTreeMap::new(),
            outgoing: VecDeque::new(),
            incoming: VecDeque::new(),
            disconnect_reason: None,
        })
    }

    pub fn state(&self) -> LspState {
        self.session.state()
    }

    pub fn server_capabilities(&self) -> Option<&Value> {
        self.session.server_capabilities()
    }

    /// Queue `initialize`. On failure or timeout the session cannot be reused.
    pub fn initialize(
        &mut self,
        params: Map<String, Value>,
        timeout: Duration,
    ) -> Result<RequestId, TransportError> {
        let (id, message) = self.session.initialize(params)?;
        self.queue_request(id, message, timeout)
    }

    /// Queue a request. The deadline starts when this call queues the request.
    /// A timeout frees its pending slot. The server can still send a late reply.
    pub fn request(
        &mut self,
        method: impl Into<String>,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<RequestId, TransportError> {
        let (id, message) = self.session.request(method, params)?;
        self.queue_request(id, message, timeout)
    }

    pub fn notify(
        &mut self,
        method: impl Into<String>,
        params: Option<Value>,
    ) -> Result<(), TransportError> {
        let message = self.session.notification(method, params)?;
        self.queue(message, None)
    }

    pub fn reply(
        &mut self,
        id: RpcId,
        outcome: Result<Value, ResponseError>,
    ) -> Result<(), TransportError> {
        if !matches!(self.state(), LspState::Initializing(_) | LspState::Ready) {
            return Err(RpcError::InvalidSessionState.into());
        }
        self.queue(
            RpcMessage::Response {
                id: Some(id),
                outcome,
            },
            None,
        )
    }

    /// Queue `$/cancelRequest` once per pending request. Cancellation does not
    /// settle the request; a response or the deadline still settles it.
    pub fn cancel(&mut self, id: RequestId) -> Result<(), TransportError> {
        self.check_queue()?;
        if let Some(message) = self.session.cancel(id)? {
            self.push(message, Some(id))?;
        }
        Ok(())
    }

    /// Drive both streams and wait for up to `wait` for the next event. A zero
    /// wait does one nonblocking I/O pass. Deadlines are checked before receiving
    /// further messages. `None` does not close the connection.
    pub fn next_event(&mut self, wait: Duration) -> Result<Option<TransportEvent>, TransportError> {
        let wait = Deadline::new(wait);
        if matches!(self.state(), LspState::Closed | LspState::Failed) {
            return Err(RpcError::InvalidSessionState.into());
        }
        loop {
            let now = Instant::now();
            let earliest = self
                .deadlines
                .iter()
                .map(|(&id, deadline)| (deadline.remaining(now), id))
                .min();
            if let Some((Duration::ZERO, id)) = earliest {
                self.session.abort(id)?;
                self.deadlines.remove(&id);
                // A partial frame must still be written in full. Remove only
                // frames that have not started, including unsent cancellations.
                self.outgoing
                    .retain(|frame| frame.id != Some(id) || frame.written != 0);
                self.stop_if_failed();
                return Ok(Some(TransportEvent::TimedOut(id)));
            }
            let mut progress = false;
            if self.disconnect_reason.is_none() {
                if self.incoming.is_empty() {
                    progress = self.read_once();
                }
                if self.disconnect_reason.is_none() {
                    match self.write_once() {
                        Ok(written) => progress |= written,
                        Err(error) => self.disconnect_reason = Some(error),
                    }
                }
            }
            if let Some(message) = self.incoming.pop_front() {
                return self.handle_message(message).map(Some);
            }
            if let Some(reason) = self.disconnect_reason.take() {
                let pending = self.session.close();
                self.deadlines.clear();
                self.stop();
                return Ok(Some(TransportEvent::Disconnected { reason, pending }));
            }
            let remaining = wait.remaining(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            if !progress {
                let delay = remaining.min(IDLE_INTERVAL);
                thread::sleep(earliest.map_or(delay, |(timeout, _)| delay.min(timeout)));
            }
        }
    }

    /// Send `shutdown`, wait for a null result, then send `exit` and wait for
    /// the child under the same deadline. This consumes the connection and
    /// discards any unrelated events. On failure, Drop terminates the child.
    pub fn shutdown(mut self, timeout: Duration) -> Result<ExitStatus, TransportError> {
        let deadline = Deadline::new(timeout);
        let id = self.request("shutdown", None, timeout)?;
        loop {
            let remaining = deadline.remaining(Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::ShutdownTimedOut);
            }
            match self.next_event(remaining)? {
                Some(TransportEvent::Resolution(resolution)) if resolution.id == id => {
                    let result = resolution
                        .outcome
                        .map_err(TransportError::ShutdownRejected)?;
                    if !result.is_null() {
                        return Err(TransportError::InvalidShutdownResult);
                    }
                    break;
                }
                Some(TransportEvent::Disconnected { reason, .. }) => return Err(reason),
                Some(TransportEvent::TimedOut(expired)) if expired == id => {
                    return Err(TransportError::ShutdownTimedOut);
                }
                _ => {}
            }
        }
        self.notify("exit", None)?;
        self.session.close();
        loop {
            let remaining = deadline.remaining(Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::ShutdownTimedOut);
            }
            let mut progress = self.write_once()?;
            if self.outgoing.is_empty() {
                self.stdin.take();
                if let Some(status) = self
                    .child
                    .try_wait()
                    .map_err(|e| io_error("wait for LSP process", e))?
                {
                    return if status.success() {
                        Ok(status)
                    } else {
                        Err(TransportError::UnsuccessfulExit(status))
                    };
                }
            }
            // Drain stdout while waiting; the child need not close it before exit.
            if let Some(stdout) = self.stdout.as_mut() {
                match stdout.read(&mut [0; IO_CHUNK_BYTES]) {
                    Ok(0) => {
                        self.stdout.take();
                    }
                    Ok(_) => progress = true,
                    Err(error) if retryable(&error) => {}
                    Err(error) => return Err(io_error("read LSP stdout", error)),
                }
            }
            if !progress {
                thread::sleep(remaining.min(IDLE_INTERVAL));
            }
        }
    }

    fn queue_request(
        &mut self,
        id: RequestId,
        message: RpcMessage,
        timeout: Duration,
    ) -> Result<RequestId, TransportError> {
        if let Err(error) = self.queue(message, Some(id)) {
            self.session.abort(id)?;
            self.stop_if_failed();
            return Err(error);
        }
        self.deadlines.insert(id, Deadline::new(timeout));
        Ok(id)
    }

    fn check_queue(&self) -> Result<(), TransportError> {
        if self.outgoing.len() >= MAX_QUEUED {
            return Err(TransportError::QueueFull);
        }
        Ok(())
    }

    fn queue(&mut self, message: RpcMessage, id: Option<RequestId>) -> Result<(), TransportError> {
        self.check_queue()?;
        self.push(message, id)
    }

    fn push(&mut self, message: RpcMessage, id: Option<RequestId>) -> Result<(), TransportError> {
        let bytes = self.codec.encode(&message.into())?;
        self.outgoing.push_back(Outbound {
            bytes,
            id,
            written: 0,
        });
        Ok(())
    }

    fn handle_message(
        &mut self,
        message: Result<RpcMessage, RpcError>,
    ) -> Result<TransportEvent, TransportError> {
        let event = match message.and_then(|message| self.session.receive(message)) {
            Ok(LspEvent::Initialized(notification)) => {
                self.deadlines.clear();
                // Reserve room for this one notification regardless of host traffic.
                self.push(notification, None)?;
                TransportEvent::Initialized
            }
            Ok(LspEvent::Resolution(resolution)) => {
                self.deadlines.remove(&resolution.id);
                TransportEvent::Resolution(resolution)
            }
            Ok(LspEvent::Message(message)) => TransportEvent::Message(message),
            Err(error) => TransportEvent::ProtocolError(error),
        };
        self.stop_if_failed();
        Ok(event)
    }

    fn read_once(&mut self) -> bool {
        let mut input = [0; IO_CHUNK_BYTES];
        match self.stdout.as_mut().unwrap().read(&mut input) {
            Ok(0) => {
                self.disconnect_reason = Some(match self.codec.finish() {
                    Ok(()) => TransportError::Eof(self.child.try_wait().ok().flatten()),
                    Err(error) => error.into(),
                });
                false
            }
            Ok(count) => {
                let incoming = &mut self.incoming;
                if let Err(error) = self.codec.feed(&input[..count], |body| {
                    incoming.push_back(RpcMessage::from_json(body));
                }) {
                    self.disconnect_reason = Some(error.into());
                }
                true
            }
            Err(error) if retryable(&error) => false,
            Err(error) => {
                self.disconnect_reason = Some(io_error("read LSP stdout", error));
                false
            }
        }
    }

    fn write_once(&mut self) -> Result<bool, TransportError> {
        let Some(frame) = self.outgoing.front_mut() else {
            return Ok(false);
        };
        let remaining = &frame.bytes[frame.written..];
        match self
            .stdin
            .as_mut()
            .unwrap()
            .write(&remaining[..remaining.len().min(IO_CHUNK_BYTES)])
        {
            Ok(0) => Err(io_error("write LSP stdin", io::ErrorKind::WriteZero.into())),
            Ok(count) => {
                frame.written += count;
                if frame.written == frame.bytes.len() {
                    self.outgoing.pop_front();
                }
                Ok(true)
            }
            Err(error) if retryable(&error) => Ok(false),
            Err(error) => Err(io_error("write LSP stdin", error)),
        }
    }

    fn stop_if_failed(&mut self) {
        if self.state() == LspState::Failed {
            self.deadlines.clear();
            self.stop();
        }
    }

    fn stop(&mut self) {
        self.stdin.take();
        self.stdout.take();
        self.outgoing.clear();
        self.incoming.clear();
        if !matches!(self.child.try_wait(), Ok(Some(_))) && self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Outbound {
    bytes: Vec<u8>,
    id: Option<RequestId>,
    written: usize,
}

struct Deadline {
    start: Instant,
    timeout: Duration,
}

impl Deadline {
    fn new(timeout: Duration) -> Self {
        Self {
            start: Instant::now(),
            timeout,
        }
    }

    fn remaining(&self, now: Instant) -> Duration {
        self.timeout.saturating_sub(now.duration_since(self.start))
    }
}

fn stdio_pair() -> Result<(UnixStream, Stdio), TransportError> {
    let (parent, child) = UnixStream::pair().map_err(|e| io_error("create LSP stdio pair", e))?;
    parent
        .set_nonblocking(true)
        .map_err(|e| io_error("set LSP stdio nonblocking", e))?;
    Ok((parent, Stdio::from(OwnedFd::from(child))))
}

fn io_error(operation: &'static str, error: io::Error) -> TransportError {
    TransportError::Io { operation, error }
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}
