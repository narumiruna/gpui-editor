//! Local Developer Mode protocol. Only the worker thread does socket I/O.

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Write},
    sync::{Arc, Mutex},
    thread::{self, Thread},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
#[path = "bridge/macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "bridge/windows.rs"]
mod platform;

use platform::{Listener, Stream};

const MAX_FRAME: usize = 6 * 1024 * 1024;

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

const VERSION: u32 = 1;
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

impl Bridge {
    pub(super) fn start() -> io::Result<Self> {
        Self::spawn(Listener::start()?)
    }

    #[cfg(test)]
    pub(super) fn listen(base: &std::path::Path) -> io::Result<Self> {
        Self::spawn(Listener::listen(base)?)
    }

    fn spawn(listener: Listener) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(State::default()));
        let worker_state = state.clone();
        let worker = thread::Builder::new()
            .name("developer-bridge".into())
            .spawn(move || {
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

fn serve(mut listener: Listener, shared: &SharedState) {
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
            let mut socket = match listener.accept() {
                Ok(accepted) => accepted,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break 'serve,
            };
            progress = true;
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
    input: BufReader<Stream>,
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
    fn new(socket: Stream) -> Self {
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
            .get_mut()
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

fn encode(message: &impl Serialize) -> io::Result<Vec<u8>> {
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
pub(super) fn test_directory() -> io::Result<tempfile::TempDir> {
    platform::test_directory()
}

#[cfg(test)]
pub(super) struct TestClient(Stream);

#[cfg(test)]
impl TestClient {
    pub(super) fn connect(base: &std::path::Path) -> io::Result<BufReader<Self>> {
        Self::retry(|| platform::test_client(base)).map(|stream| BufReader::new(Self(stream)))
    }

    fn retry<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match operation() {
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(POLL_INTERVAL);
                }
                result => return result,
            }
        }
    }
}

#[cfg(test)]
impl std::io::Read for TestClient {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Self::retry(|| self.0.read(buf))
    }
}

#[cfg(test)]
impl Write for TestClient {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Self::retry(|| self.0.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const PAIR: &[u8] = b"{\"type\":\"pair\",\"version\":1,\"project\":\"/test\",\"session\":\"practice\",\"code\":\"012345\"}\n";
    const TEST_TIMEOUT: Duration = Duration::from_secs(3);

    struct Fixture {
        bridge: Bridge,
        temp: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = platform::test_directory().unwrap();
            let bridge = Bridge::listen(temp.path()).unwrap();
            Self { bridge, temp }
        }

        fn connect(&self) -> BufReader<TestClient> {
            TestClient::connect(self.temp.path()).unwrap()
        }
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

    fn read_message(socket: &mut BufReader<TestClient>) -> Value {
        let mut line = String::new();
        socket.read_line(&mut line).unwrap();
        assert!(line.ends_with('\n'), "missing server frame: {line:?}");
        serde_json::from_str(&line).unwrap()
    }

    fn write_message(socket: &mut BufReader<TestClient>, value: Value) {
        writeln!(socket.get_mut(), "{value}").unwrap();
    }

    fn pair(fixture: &Fixture, socket: &mut BufReader<TestClient>) -> u64 {
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

    fn approve(fixture: &Fixture, socket: &mut BufReader<TestClient>, id: u64) {
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
    fn feedback_can_exceed_the_transport_buffer() {
        let fixture = Fixture::new();
        let mut socket = fixture.connect();
        let id = pair(&fixture, &mut socket);
        approve(&fixture, &mut socket, id);

        let data = "A".repeat(256 * 1024);
        let feedback = fixture
            .bridge
            .send_feedback(
                target(),
                "large attachment".into(),
                Some(Attachment {
                    mime_type: "image/png".into(),
                    data: data.clone(),
                }),
            )
            .unwrap();
        let reply = read_message(&mut socket);
        assert_eq!(reply["id"], feedback);
        assert!(reply["image"]["data"].as_str() == Some(data.as_str()));
        write_message(
            &mut socket,
            json!({"type": "receipt", "id": feedback, "accepted": true}),
        );
        assert!(
            matches!(wait_until(|| fixture.bridge.try_event()), Event::Receipt { id, accepted: true, .. } if id == feedback)
        );
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
    fn frame_limit_includes_lf_for_writes() {
        let mut message = json!({"text": ""});
        let empty_size = encode(&message).unwrap().len();
        message["text"] = "a".repeat(MAX_FRAME - empty_size).into();
        let frame = encode(&message).unwrap();
        assert_eq!(frame.len(), MAX_FRAME);
        assert_eq!(frame.last(), Some(&b'\n'));
        message["text"] = "a".repeat(MAX_FRAME - empty_size + 1).into();
        assert!(encode(&message).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn frame_limit_includes_lf_for_reads() {
        let (mut sender, socket) = Stream::pair().unwrap();
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
    }
}
