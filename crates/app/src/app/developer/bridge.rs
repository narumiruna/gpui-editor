//! Local Developer Mode transport. Only the macOS worker thread does socket I/O.

use serde::Serialize;
use std::io;
#[cfg(any(target_os = "macos", test))]
use std::io::Write;

#[cfg(target_os = "macos")]
#[path = "bridge/macos.rs"]
mod platform;

pub(super) use platform::Bridge;

#[cfg(any(target_os = "macos", test))]
const MAX_FRAME: usize = 6 * 1024 * 1024;

// The UI handles all events, including on platforms without a pairing transport.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub(in super::super) struct Bridge;

    impl Bridge {
        pub(in super::super) fn start() -> io::Result<Self> {
            Err(unsupported())
        }

        pub(in super::super) fn try_event(&self) -> Option<Event> {
            None
        }

        pub(in super::super) fn decide(&self, _id: u64, _approved: bool) {}

        pub(in super::super) fn unpair(&self) {}

        pub(in super::super) fn send_feedback(
            &self,
            _target: Target,
            _text: String,
            _image: Option<Attachment>,
        ) -> io::Result<u32> {
            Err(unsupported())
        }

        pub(in super::super) fn record(&self, _event: &'static str) {}
    }

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Pi pairing is not supported on Windows.",
        )
    }
}

#[cfg(any(target_os = "macos", test))]
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

#[cfg(any(target_os = "macos", test))]
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn pairing_and_feedback_are_explicitly_unsupported() {
        let error = Bridge::start().err().expect("pairing must be unsupported");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert_eq!(error.to_string(), "Pi pairing is not supported on Windows.");

        let bridge = Bridge;
        let target = Target {
            id: "save".into(),
            label: "Save".into(),
            bounds: [1.0, 2.0, 3.0, 4.0],
        };
        let error = bridge
            .send_feedback(target, "test".into(), None)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert_eq!(error.to_string(), "Pi pairing is not supported on Windows.");
        assert!(bridge.try_event().is_none());
    }
}
