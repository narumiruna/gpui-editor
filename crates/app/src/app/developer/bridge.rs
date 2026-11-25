//! Local Developer Mode transport. Only the worker thread does socket I/O.

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    ffi::OsStr,
    fs::{self, DirBuilder, Permissions},
    io::{self, BufRead, BufReader, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::{self, Thread},
    time::{Duration, Instant},
};

const VERSION: u32 = 1;
const MAX_FRAME: usize = 6 * 1024 * 1024;
const MAX_EVENTS: usize = 32;
const MAX_LOGS: usize = 128;
const MAX_LOG_BYTES: usize = 512;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(30);
const BUSY: &[u8] = b"{\"type\":\"rejected\",\"message\":\"Editor is busy\"}\n";

type SharedState = Mutex<State>;

pub(super) struct Bridge {
    state: Arc<SharedState>,
    worker: Thread,
}

#[derive(Debug)]
pub(super) enum Event {
    Pair {
        id: u64,
        project: String,
        session: String,
        code: String,
    },
    Paired {
        id: u64,
    },
    Disconnected {
        id: u64,
    },
    Receipt {
        id: u32,
        accepted: bool,
        message: Option<String>,
    },
}

#[derive(Clone, Serialize)]
pub(super) struct Target {
    pub id: String,
    pub label: String,
    pub bounds: [f32; 4],
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Attachment {
    pub mime_type: String,
    pub data: String,
}

impl Bridge {
    pub(super) fn start() -> io::Result<Self> {
        Self::listen(&discovery_base(&darwin_temp_dir()?)?)
    }

    pub(super) fn listen(base: &Path) -> io::Result<Self> {
        let instance = tempfile::Builder::new()
            .prefix("i-")
            .permissions(Permissions::from_mode(0o700))
            .tempdir_in(base)?;
        check_private(instance.path(), false)?;
        let path = instance.path().join("s");
        // Darwin's 104-byte sun_path must also fit the terminating NUL.
        if path.as_os_str().as_bytes().len() >= 104 {
            return Err(invalid("Developer socket path is too long"));
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, Permissions::from_mode(0o600))?;
        check_private(&path, true)?;
        listener.set_nonblocking(true)?;

        let state = Arc::new(Mutex::new(State::default()));
        let worker_state = state.clone();
        let worker = thread::Builder::new()
            .name("developer-bridge".into())
            .spawn(move || {
                let _instance = instance;
                serve(listener, &worker_state);
            })?
            .thread()
            .clone();
        Ok(Self { state, worker })
    }

    pub(super) fn try_event(&self) -> Option<Event> {
        self.state.lock().unwrap().events.pop_front()
    }

    pub(super) fn decide(&self, id: u64, approved: bool) {
        if let Some(peer) = &mut self.state.lock().unwrap().peer
            && peer.id == id
            && peer.status == Status::Pending
        {
            peer.status = if approved {
                Status::Approved
            } else {
                Status::Rejected
            };
        }
        self.worker.unpark();
    }

    pub(super) fn unpair(&self) {
        if let Some(peer) = &mut self.state.lock().unwrap().peer {
            peer.status = Status::Revoked;
        }
        self.worker.unpark();
    }

    /// Queues feedback without encoding or writing it on the caller's thread.
    /// Poll the receipt before sending another item.
    pub(super) fn send_feedback(
        &self,
        target: Target,
        text: String,
        image: Option<Attachment>,
    ) -> io::Result<u32> {
        let raw_bytes = target
            .id
            .len()
            .saturating_add(target.label.len())
            .saturating_add(text.len())
            .saturating_add(image.as_ref().map_or(0, |image| image.data.len()));
        if raw_bytes >= MAX_FRAME
            || !target.bounds.iter().all(|bound| bound.is_finite())
            || image
                .as_ref()
                .is_some_and(|image| image.mime_type != "image/png")
        {
            return Err(invalid("Invalid feedback attachment, bounds, or size"));
        }

        let mut state = self.state.lock().unwrap();
        if state.stopped
            || !state
                .peer
                .as_ref()
                .is_some_and(|peer| peer.status == Status::Paired)
        {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "No paired session",
            ));
        }
        // Reserve space for the receipt and disconnect. Keep at most one unpolled
        // receipt, since its message may be as large as the frame limit.
        if state.events.len() > MAX_EVENTS - 2
            || state
                .events
                .iter()
                .any(|event| matches!(event, Event::Receipt { .. }))
            || state.peer.as_ref().unwrap().feedback.is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Feedback is busy",
            ));
        }
        let id = state.next_feedback;
        state.next_feedback = id.wrapping_add(1);
        state.peer.as_mut().unwrap().feedback = Some(FeedbackSlot::Queued(Feedback {
            id,
            target,
            text,
            image,
        }));
        drop(state);
        self.worker.unpark();
        Ok(id)
    }

    /// Accept only static app events, never document or peer data.
    pub(super) fn record(&self, event: &'static str) {
        let mut state = self.state.lock().unwrap();
        if state.logs.len() == MAX_LOGS {
            state.logs.pop_front();
        }
        state
            .logs
            .push_back(&event[..event.floor_char_boundary(MAX_LOG_BYTES)]);
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.state.lock().unwrap().stopped = true;
        self.worker.unpark();
    }
}

fn darwin_temp_dir() -> io::Result<PathBuf> {
    let mut buffer = [0_u8; libc::PATH_MAX as usize];
    // SAFETY: confstr writes at most buffer.len() bytes to this writable buffer.
    let size = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if !(2..=buffer.len()).contains(&size) || buffer[size - 1] != 0 {
        return Err(invalid("Cannot locate the Darwin user temporary directory"));
    }
    let path = Path::new(OsStr::from_bytes(&buffer[..size - 1]));
    if !path.is_absolute() {
        return Err(invalid("Darwin user temporary directory is not absolute"));
    }
    // Strip the trailing slash before checking that the final component is not
    // a symlink. Canonicalize system aliases such as /var for discovery.
    let path: PathBuf = path.components().collect();
    check_private(&path, false)?;
    path.canonicalize()
}

fn discovery_base(temp: &Path) -> io::Result<PathBuf> {
    check_private(temp, false)?;
    let base = temp.join("gpui-pi");
    match DirBuilder::new().mode(0o700).create(&base) {
        Ok(()) => (),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err),
    }
    check_private(&base, false)?;
    Ok(base)
}

fn check_private(path: &Path, socket: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let (correct_type, mode) = if socket {
        (meta.file_type().is_socket(), 0o600)
    } else {
        (meta.is_dir(), 0o700)
    };
    if !correct_type || meta.uid() != effective_uid() || meta.mode() & 0o7777 != mode {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unsafe Developer Mode discovery directory or socket",
        ));
    }
    Ok(())
}

fn effective_uid() -> libc::uid_t {
    // SAFETY: geteuid takes no arguments and has no memory preconditions.
    unsafe { libc::geteuid() }
}

fn same_user(socket: &UnixStream) -> bool {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: the descriptor stays open for this call, and both outputs are writable.
    let result = unsafe { libc::getpeereid(socket.as_raw_fd(), &mut uid, &mut gid) };
    result == 0 && uid == effective_uid()
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMessage {
    Pair {
        version: u32,
        project: String,
        session: String,
        code: String,
    },
    Logs {
        id: u32,
        limit: u32,
    },
    Receipt {
        id: u32,
        accepted: bool,
        message: Option<String>,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ServerMessage {
    Paired { version: u32 },
    Rejected { message: &'static str },
    Logs { id: u32, entries: Vec<&'static str> },
    Feedback(Feedback),
}

#[derive(Serialize)]
struct Feedback {
    id: u32,
    target: Target,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<Attachment>,
}

#[derive(Default)]
struct State {
    stopped: bool,
    peer: Option<Peer>,
    events: VecDeque<Event>,
    logs: VecDeque<&'static str>,
    next_feedback: u32,
}

struct Peer {
    id: u64,
    status: Status,
    shown: bool,
    feedback: Option<FeedbackSlot>,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Status {
    Handshake,
    Pending,
    Approved,
    Rejected,
    Paired,
    Revoked,
}

enum FeedbackSlot {
    Queued(Feedback),
    Awaiting(u32),
}

fn serve(listener: UnixListener, shared: &SharedState) {
    let mut next_peer = 0_u64;
    let mut connection: Option<Connection> = None;
    'serve: while !shared.lock().unwrap().stopped {
        let mut progress = false;
        if let Some(peer) = &mut connection {
            match peer.advance(shared) {
                Ok(Step::Idle) => (),
                Ok(Step::Progress) => progress = true,
                Ok(Step::Close) | Err(_) => {
                    connection = None;
                    disconnect(shared);
                    progress = true;
                }
            }
        }
        // Limit accepts per iteration so busy clients cannot starve the active one.
        for _ in 0..8 {
            let (mut socket, _) = match listener.accept() {
                Ok(accepted) => accepted,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break 'serve,
            };
            progress = true;
            if !same_user(&socket) || socket.set_nonblocking(true).is_err() {
                continue;
            }
            let mut state = shared.lock().unwrap();
            if state.stopped {
                break 'serve;
            }
            // Reserve space for pair, paired, and disconnect before accepting.
            if connection.is_some() || state.events.len() > MAX_EVENTS - 3 {
                drop(state);
                // Best effort; a fresh socket can normally buffer this small reply.
                let _ = socket.write(BUSY);
                continue;
            }
            state.peer = Some(Peer {
                id: next_peer,
                status: Status::Handshake,
                shown: false,
                feedback: None,
            });
            next_peer = next_peer.wrapping_add(1);
            connection = Some(Connection::new(socket));
            // Give a closed, request-free discovery probe a chance to clear
            // before accepting the actual pairing connection behind it.
            break;
        }
        if !progress {
            thread::park_timeout(POLL_INTERVAL);
        }
    }
    drop(connection);
    shared.lock().unwrap().stopped = true;
    disconnect(shared);
}

fn disconnect(shared: &SharedState) {
    let mut state = shared.lock().unwrap();
    let peer = state.peer.take();
    if let Some(peer) = &peer
        && peer.shown
    {
        state.events.push_back(Event::Disconnected { id: peer.id });
    }
    // A queued attachment, if any, can be freed outside the lock.
    drop(state);
}

struct Connection {
    input: BufReader<UnixStream>,
    frame: Vec<u8>,
    frame_deadline: Option<Instant>,
    pair_deadline: Instant,
    output: Option<Output>,
    receipt_deadline: Option<Instant>,
}

struct Output {
    bytes: Vec<u8>,
    written: usize,
    deadline: Instant,
    after: AfterWrite,
}

#[derive(Clone, Copy)]
enum AfterWrite {
    Paired,
    Close,
    Logs,
    Feedback,
}

enum Step {
    Idle,
    Progress,
    Close,
}

enum Input {
    Idle,
    Progress,
    Message(ClientMessage),
}

impl Connection {
    fn new(socket: UnixStream) -> Self {
        Self {
            input: BufReader::new(socket),
            frame: Vec::new(),
            frame_deadline: None,
            pair_deadline: Instant::now() + IO_TIMEOUT,
            output: None,
            receipt_deadline: None,
        }
    }

    fn advance(&mut self, shared: &SharedState) -> io::Result<Step> {
        let status = shared.lock().unwrap().peer.as_ref().unwrap().status;
        let now = Instant::now();
        if status == Status::Revoked
            || self
                .receipt_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            return Ok(Step::Close);
        }
        if self.output.is_some() {
            return self.flush_output(shared);
        }
        match status {
            Status::Handshake | Status::Pending | Status::Approved if now >= self.pair_deadline => {
                self.reject("Pairing timed out")
            }
            Status::Approved => self.reply(
                ServerMessage::Paired { version: VERSION },
                AfterWrite::Paired,
            ),
            Status::Rejected => self.reject("Pairing declined"),
            _ => {
                if status == Status::Paired
                    && let Some(feedback) = take_feedback(shared)
                {
                    let id = feedback.id;
                    let valid = feedback
                        .image
                        .as_ref()
                        .is_none_or(|image| valid_base64(&image.data));
                    if valid
                        && self
                            .reply(ServerMessage::Feedback(feedback), AfterWrite::Feedback)
                            .is_ok()
                    {
                        return Ok(Step::Progress);
                    }
                    finish_feedback(
                        shared,
                        id,
                        false,
                        Some("Invalid or oversized feedback frame".into()),
                    );
                    return Ok(Step::Progress);
                }
                match self.read_input() {
                    Ok(Input::Idle) => Ok(Step::Idle),
                    Ok(Input::Progress) => Ok(Step::Progress),
                    Ok(Input::Message(message)) => self.handle(shared, status, message),
                    Err(err)
                        if err.kind() == io::ErrorKind::InvalidData && status != Status::Paired =>
                    {
                        self.reject("Invalid pairing request")
                    }
                    Err(err) => Err(err),
                }
            }
        }
    }

    fn handle(
        &mut self,
        shared: &SharedState,
        status: Status,
        message: ClientMessage,
    ) -> io::Result<Step> {
        match (status, message) {
            (
                Status::Handshake,
                ClientMessage::Pair {
                    version: VERSION,
                    project,
                    session,
                    code,
                },
            ) if project.len() <= 512
                && session.len() <= 128
                && code.len() == 6
                && code.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                let mut state = shared.lock().unwrap();
                if state.stopped {
                    return Ok(Step::Close);
                }
                let peer = state.peer.as_mut().unwrap();
                if peer.status != Status::Handshake {
                    return Ok(Step::Close);
                }
                let id = peer.id;
                peer.status = Status::Pending;
                peer.shown = true;
                state.events.push_back(Event::Pair {
                    id,
                    project,
                    session,
                    code,
                });
                self.pair_deadline = Instant::now() + CONSENT_TIMEOUT;
                Ok(Step::Progress)
            }
            (Status::Handshake, _) => self.reject("Invalid pairing request"),
            (Status::Pending, _) => self.reject("Pairing consent is pending"),
            (Status::Paired, ClientMessage::Logs { id, limit }) => {
                let state = shared.lock().unwrap();
                let start = state.logs.len().saturating_sub(limit as usize);
                let entries = state.logs.iter().skip(start).copied().collect();
                drop(state);
                self.reply(ServerMessage::Logs { id, entries }, AfterWrite::Logs)
            }
            (
                Status::Paired,
                ClientMessage::Receipt {
                    id,
                    accepted,
                    message,
                },
            ) => {
                if self.receipt_deadline.is_some() && finish_feedback(shared, id, accepted, message)
                {
                    self.receipt_deadline = None;
                    Ok(Step::Progress)
                } else {
                    Ok(Step::Close)
                }
            }
            _ => Ok(Step::Close),
        }
    }

    fn read_input(&mut self) -> io::Result<Input> {
        let now = Instant::now();
        if self.frame_deadline.is_some_and(|deadline| now >= deadline) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let bytes = match self.input.fill_buf() {
            Ok([]) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(bytes) => bytes,
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(Input::Idle);
            }
            Err(err) => return Err(err),
        };
        let newline = bytes.iter().position(|&byte| byte == b'\n');
        let count = newline.map_or(bytes.len(), |index| index + 1);
        let size = self.frame.len() + count;
        if size > MAX_FRAME || (size == MAX_FRAME && newline.is_none()) {
            return Err(invalid("Client frame is too large"));
        }
        self.frame.extend_from_slice(&bytes[..count]);
        self.input.consume(count);
        if newline.is_none() {
            self.frame_deadline.get_or_insert(now + IO_TIMEOUT);
            return Ok(Input::Progress);
        }
        let message = std::str::from_utf8(&self.frame)
            .ok()
            .and_then(|frame| serde_json::from_str(frame).ok())
            .ok_or_else(|| invalid("Invalid UTF-8 JSON frame"));
        self.frame.clear();
        self.frame_deadline = None;
        message.map(Input::Message)
    }

    fn reject(&mut self, message: &'static str) -> io::Result<Step> {
        self.reply(ServerMessage::Rejected { message }, AfterWrite::Close)
    }

    fn reply(&mut self, message: ServerMessage, after: AfterWrite) -> io::Result<Step> {
        self.output = Some(Output {
            bytes: encode(&message)?,
            written: 0,
            deadline: Instant::now() + IO_TIMEOUT,
            after,
        });
        Ok(Step::Progress)
    }

    fn flush_output(&mut self, shared: &SharedState) -> io::Result<Step> {
        let output = self.output.as_mut().unwrap();
        if Instant::now() >= output.deadline {
            return Ok(Step::Close);
        }
        // Serialize outside the lock. Serialize revocation against only one
        // bounded, nonblocking write so no new bytes are written after unpair.
        let mut state = shared.lock().unwrap();
        if state.stopped || state.peer.as_ref().unwrap().status == Status::Revoked {
            return Ok(Step::Close);
        }
        let end = (output.written + 64 * 1024).min(output.bytes.len());
        match self
            .input
            .get_ref()
            .write(&output.bytes[output.written..end])
        {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => output.written += count,
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(Step::Idle);
            }
            Err(err) => return Err(err),
        }
        if output.written < output.bytes.len() {
            return Ok(Step::Progress);
        }
        match output.after {
            AfterWrite::Close => return Ok(Step::Close),
            AfterWrite::Paired => {
                let peer = state.peer.as_mut().unwrap();
                peer.status = Status::Paired;
                let id = peer.id;
                state.events.push_back(Event::Paired { id });
            }
            AfterWrite::Feedback => self.receipt_deadline = Some(Instant::now() + RECEIPT_TIMEOUT),
            AfterWrite::Logs => (),
        }
        drop(state);
        self.output = None;
        Ok(Step::Progress)
    }
}

fn take_feedback(shared: &SharedState) -> Option<Feedback> {
    let mut state = shared.lock().unwrap();
    let peer = state.peer.as_mut().unwrap();
    if peer.status != Status::Paired {
        return None;
    }
    match peer.feedback.take() {
        Some(FeedbackSlot::Queued(feedback)) => {
            peer.feedback = Some(FeedbackSlot::Awaiting(feedback.id));
            Some(feedback)
        }
        other => {
            peer.feedback = other;
            None
        }
    }
}

fn finish_feedback(shared: &SharedState, id: u32, accepted: bool, message: Option<String>) -> bool {
    let mut state = shared.lock().unwrap();
    let peer = state.peer.as_mut().unwrap();
    if peer.status != Status::Paired
        || !matches!(peer.feedback, Some(FeedbackSlot::Awaiting(current)) if current == id)
    {
        return false;
    }
    peer.feedback = None;
    state.events.push_back(Event::Receipt {
        id,
        accepted,
        message,
    });
    true
}

fn valid_base64(data: &str) -> bool {
    let body = data
        .strip_suffix("==")
        .or_else(|| data.strip_suffix('='))
        .unwrap_or(data);
    !body.is_empty()
        && data.len().is_multiple_of(4)
        && body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
}

fn encode(message: &ServerMessage) -> io::Result<Vec<u8>> {
    #[derive(Default)]
    struct Capped(Vec<u8>);

    impl Write for Capped {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_FRAME - 1 - self.0.len() {
                return Err(invalid("Server frame is too large"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut frame = Capped::default();
    serde_json::to_writer(&mut frame, message).map_err(io::Error::other)?;
    frame.0.push(b'\n');
    Ok(frame.0)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::os::unix::fs::symlink;

    const PAIR: &[u8] = b"{\"type\":\"pair\",\"version\":1,\"project\":\"/test\",\"session\":\"practice\",\"code\":\"012345\"}\n";
    const TEST_TIMEOUT: Duration = Duration::from_secs(3);

    struct Fixture {
        bridge: Bridge,
        path: PathBuf,
        _temp: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = sandbox();
            let base = discovery_base(temp.path()).unwrap();
            let bridge = Bridge::listen(&base).unwrap();
            let path = fs::read_dir(&base)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
                .join("s");
            Self {
                bridge,
                path,
                _temp: temp,
            }
        }

        fn connect(&self) -> BufReader<UnixStream> {
            let stream = UnixStream::connect(&self.path).unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            BufReader::new(stream)
        }
    }

    fn sandbox() -> tempfile::TempDir {
        tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o700))
            .tempdir_in(darwin_temp_dir().unwrap())
            .unwrap()
    }

    fn wait_until<T>(mut poll: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            if let Some(value) = poll() {
                return value;
            }
            assert!(Instant::now() < deadline, "timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn read_message(socket: &mut BufReader<UnixStream>) -> Value {
        let mut line = String::new();
        socket.read_line(&mut line).unwrap();
        assert!(line.ends_with('\n'), "missing server frame: {line:?}");
        serde_json::from_str(&line).unwrap()
    }

    fn write_message(socket: &mut BufReader<UnixStream>, value: Value) {
        writeln!(socket.get_mut(), "{value}").unwrap();
    }

    fn pair(fixture: &Fixture, socket: &mut BufReader<UnixStream>) -> u64 {
        socket.get_mut().write_all(&PAIR[..24]).unwrap();
        socket.get_mut().write_all(&PAIR[24..]).unwrap();
        match wait_until(|| fixture.bridge.try_event()) {
            Event::Pair {
                id,
                project,
                session,
                code,
            } => {
                assert_eq!(
                    (project.as_str(), session.as_str(), code.as_str()),
                    ("/test", "practice", "012345")
                );
                id
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    fn approve(fixture: &Fixture, socket: &mut BufReader<UnixStream>, id: u64) {
        fixture.bridge.decide(id, true);
        assert_eq!(
            read_message(socket),
            json!({"type": "paired", "version": 1})
        );
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Paired { id: found } if found == id)
        );
    }

    fn target() -> Target {
        Target {
            id: "save".into(),
            label: "Save".into(),
            bounds: [1.0, 2.0, 3.0, 4.0],
        }
    }

    #[test]
    fn discovery_rejects_symlinks_and_bad_modes_and_removes_only_its_instance() {
        let fixture = Fixture::new();
        let instance = fixture.path.parent().unwrap().to_path_buf();
        let base = instance.parent().unwrap();
        check_private(&instance, false).unwrap();
        check_private(&fixture.path, true).unwrap();
        let sibling = tempfile::tempdir_in(base).unwrap();
        let other = sandbox();
        let link = other.path().join("gpui-pi");
        symlink(base, &link).unwrap();
        assert_eq!(
            discovery_base(other.path()).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_file(&link).unwrap();
        fs::create_dir(&link).unwrap();
        fs::set_permissions(&link, Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            discovery_base(other.path()).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        drop(fixture.bridge);
        wait_until(|| (!instance.exists()).then_some(()));
        assert!(sibling.path().is_dir());
    }

    #[test]
    fn closed_probe_does_not_block_pairing_and_live_client_excludes_others() {
        let fixture = Fixture::new();
        drop(fixture.connect());
        let mut socket = fixture.connect();
        let id = pair(&fixture, &mut socket);
        fixture.bridge.decide(id + 1, true);
        assert_eq!(
            fixture
                .bridge
                .send_feedback(target(), "test".into(), None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotConnected
        );

        assert_eq!(read_message(&mut fixture.connect())["type"], "rejected");
        approve(&fixture, &mut socket, id);
        assert_eq!(read_message(&mut fixture.connect())["type"], "rejected");
        fixture.bridge.unpair();
        assert_eq!(socket.read_line(&mut String::new()).unwrap(), 0);
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Disconnected { id: found } if found == id)
        );
    }

    #[test]
    fn pending_client_cannot_read_logs_and_decline_is_reported() {
        let fixture = Fixture::new();
        fixture.bridge.record("not yet authorized");
        let mut socket = fixture.connect();
        let id = pair(&fixture, &mut socket);
        write_message(&mut socket, json!({"type": "logs", "id": 4, "limit": 10}));
        assert_eq!(read_message(&mut socket)["type"], "rejected");
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Disconnected { id: found } if found == id)
        );

        let mut socket = fixture.connect();
        let id = pair(&fixture, &mut socket);
        fixture.bridge.decide(id, false);
        assert_eq!(read_message(&mut socket)["type"], "rejected");
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Disconnected { id: found } if found == id)
        );
    }

    #[test]
    fn paired_client_gets_bounded_logs_feedback_and_matching_receipts() {
        let fixture = Fixture::new();
        fixture.bridge.record("discarded");
        for _ in 1..MAX_LOGS {
            fixture.bridge.record("older");
        }
        fixture.bridge.record("newest");
        let mut socket = fixture.connect();
        let id = pair(&fixture, &mut socket);
        approve(&fixture, &mut socket, id);

        write_message(
            &mut socket,
            json!({"type": "logs", "id": 3, "limit": u32::MAX}),
        );
        let reply = read_message(&mut socket);
        assert_eq!(reply["id"], 3);
        let entries = reply["entries"].as_array().unwrap();
        assert_eq!(entries.len(), MAX_LOGS);
        assert_eq!(entries[0], "older");
        assert_eq!(entries[MAX_LOGS - 1], "newest");
        write_message(&mut socket, json!({"type": "logs", "id": 4, "limit": 1}));
        assert_eq!(
            read_message(&mut socket),
            json!({"type": "logs", "id": 4, "entries": ["newest"]})
        );

        let feedback_id = fixture
            .bridge
            .send_feedback(target(), "test\n\"é".into(), None)
            .unwrap();
        assert_eq!(
            read_message(&mut socket),
            json!({"type": "feedback", "id": feedback_id, "target": {"id": "save", "label": "Save", "bounds": [1.0, 2.0, 3.0, 4.0]}, "text": "test\n\"é"})
        );
        assert_eq!(
            fixture
                .bridge
                .send_feedback(target(), "busy".into(), None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        write_message(
            &mut socket,
            json!({"type": "receipt", "id": feedback_id, "accepted": true}),
        );
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Receipt { id, accepted: true, message: None } if id == feedback_id)
        );

        let image = Attachment {
            mime_type: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
        };
        let second = fixture
            .bridge
            .send_feedback(target(), "with image".into(), Some(image))
            .unwrap();
        assert_eq!(
            read_message(&mut socket)["image"],
            json!({"mimeType": "image/png", "data": "iVBORw0KGgo="})
        );
        write_message(
            &mut socket,
            json!({"type": "receipt", "id": second, "accepted": false, "message": "test rejection"}),
        );
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Receipt { id, accepted: false, message: Some(message) } if id == second && message == "test rejection")
        );
        drop(socket);
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Disconnected { id: found } if found == id)
        );
    }

    #[test]
    fn pairing_rejects_malformed_utf8_version_and_oversized_identity() {
        let fixture = Fixture::new();
        for bad in [
            b"{\"type\":\"pair\",\"version\":1,\"project\":\"/test\",\"session\":\"practice\",\"code\":\"012345\",\"ignored\":\"\xff\"}\n".to_vec(),
            format!("{}\n", json!({"type": "pair", "version": 2, "project": "x", "session": "x", "code": "123456"})).into_bytes(),
            format!("{}\n", json!({"type": "pair", "version": 1, "project": "é".repeat(257), "session": "x", "code": "123456"})).into_bytes(),
            format!("{}\n", json!({"type": "pair", "version": 1, "project": "x", "session": "x", "code": "12345a"})).into_bytes(),
        ] {
            let mut socket = fixture.connect();
            socket.get_mut().write_all(&bad).unwrap();
            assert_eq!(read_message(&mut socket)["type"], "rejected");
        }
        assert!(fixture.bridge.try_event().is_none());
    }

    #[test]
    fn frame_limit_includes_lf_for_reads_and_writes() {
        let (mut sender, socket) = UnixStream::pair().unwrap();
        let mut connection = Connection::new(socket);
        connection
            .frame
            .extend_from_slice(b"{\"type\":\"logs\",\"id\":1,\"limit\":1}");
        connection.frame.resize(MAX_FRAME - 1, b' ');
        sender.write_all(b"\n").unwrap();
        assert!(matches!(
            connection.read_input().unwrap(),
            Input::Message(ClientMessage::Logs { .. })
        ));
        connection.frame.resize(MAX_FRAME - 1, b' ');
        sender.write_all(b"x").unwrap();
        assert!(
            matches!(connection.read_input(), Err(err) if err.kind() == io::ErrorKind::InvalidData)
        );

        let mut feedback = Feedback {
            id: 0,
            target: target(),
            text: String::new(),
            image: None,
        };
        let empty_size = encode(&ServerMessage::Feedback(feedback)).unwrap().len();
        feedback = Feedback {
            id: 0,
            target: target(),
            text: "a".repeat(MAX_FRAME - empty_size),
            image: None,
        };
        let mut message = ServerMessage::Feedback(feedback);
        let frame = encode(&message).unwrap();
        assert_eq!(frame.len(), MAX_FRAME);
        assert_eq!(frame.last(), Some(&b'\n'));
        if let ServerMessage::Feedback(feedback) = &mut message {
            feedback.text.push('b');
        }
        assert!(encode(&message).is_err());
    }
}
