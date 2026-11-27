use std::cell::RefCell;

use gpui_editor_codemirror::view::{
    ViewException,
    measure::{
        MAX_MEASURE_ROUNDS, MAX_PENDING_MEASURES, MeasureDrain, MeasureKey, MeasureQueue,
        MeasureQueueFull, MeasureRequest,
    },
};
use serde_json::{Value, json};

#[derive(Default)]
struct Host {
    events: RefCell<Vec<String>>,
    value: usize,
    doc: &'static str,
    repeat: Option<MeasureRequest<Self>>,
}

impl Host {
    fn push(&self, event: impl Into<String>) {
        self.events.borrow_mut().push(event.into());
    }

    fn events(&self) -> Value {
        json!(*self.events.borrow())
    }
}

fn record(label: &'static str, key: Option<MeasureKey>) -> MeasureRequest<Host> {
    MeasureRequest::new(
        key,
        move |host: &Host, _| {
            host.push(format!("read:{label}"));
            Ok(label.to_owned())
        },
        |result, host, _| {
            host.push(format!("write:{result}"));
            Ok(())
        },
    )
}

fn report(host: &Host, error: ViewException) {
    host.push(format!("error:{}", error.message));
}

fn drain(queue: &mut MeasureQueue<Host>, host: &mut Host) -> MeasureDrain {
    queue.drain(host, |_| true, report)
}

// Captured from the pinned @codemirror/view 6.44.0 in headless Chromium.
// The oracle controlled animation frames; these tests compare host-visible behavior.
fn upstream() -> Value {
    serde_json::from_str(include_str!("fixtures/view_measure.json")).unwrap()
}

#[test]
fn queues_as_one_batch_and_replaces_a_key_in_its_original_position() {
    let expected = upstream();
    let mut queue = MeasureQueue::new();
    let mut host = Host::default();
    assert!(!queue.has_pending());
    assert_eq!(drain(&mut queue, &mut host).rounds, 0);
    let middle = record("middle", None);
    for request in [
        record("superseded", Some(0)),
        middle.clone(),
        middle,
        record("replacement", Some(0)),
        record("unkeyed", None),
        record("unkeyed", None),
    ] {
        queue.requests().request(&request).unwrap();
        assert!(queue.has_pending());
    }
    assert_eq!(host.events.borrow().len(), expected["scheduling"]["before"]);
    assert_eq!(
        drain(&mut queue, &mut host),
        MeasureDrain {
            rounds: 1,
            pending: false
        }
    );
    assert!(!queue.has_pending());
    assert_eq!(host.events(), expected["scheduling"]["events"]);
}

#[test]
fn nested_requests_from_reads_and_writes_wait_for_the_next_round() {
    let mut queue = MeasureQueue::new();
    let mut host = Host {
        value: 10,
        ..Host::default()
    };
    let key = Some(1);
    queue
        .requests()
        .request(&MeasureRequest::new(
            None,
            move |host: &Host, pending| {
                host.push(format!("read:outer:{}", host.value));
                // Another request with this key belongs to the active batch, not pending.
                assert!(!pending.cancel(1));
                pending.request(&record("nested-superseded", key)).unwrap();
                Ok(host.value)
            },
            move |result, host, pending| {
                host.value += 1;
                host.push(format!("write:outer:{result}"));
                pending
                    .request(&MeasureRequest::new(
                        key,
                        |host: &Host, _| {
                            host.push(format!("read:nested:{}", host.value));
                            Ok(host.value)
                        },
                        |result, host, _| {
                            host.push(format!("write:nested:{result}"));
                            Ok(())
                        },
                    ))
                    .unwrap();
                pending.request(&record("nested-tail", None)).unwrap();
                Ok(())
            },
        ))
        .unwrap();
    queue
        .requests()
        .request(&MeasureRequest::new(
            key,
            |host: &Host, _| {
                host.push(format!("read:active:{}", host.value));
                Ok(host.value)
            },
            |result, host, _| {
                host.push(format!("write:active:{result}/{}", host.value));
                Ok(())
            },
        ))
        .unwrap();
    let mut layout_inputs = Vec::new();
    let drained = queue.drain(
        &mut host,
        |host| {
            layout_inputs.push(host.value);
            true
        },
        report,
    );
    assert_eq!(
        drained,
        MeasureDrain {
            rounds: 2,
            pending: false
        }
    );
    assert_eq!(layout_inputs, [10, 11]);
    assert_eq!(host.events(), upstream()["nested"]["events"]);
}

#[test]
fn callback_errors_do_not_skip_unrelated_requests() {
    let mut queue = MeasureQueue::new();
    let mut host = Host::default();
    for request in [
        MeasureRequest::new(
            None,
            |host: &Host, pending| -> Result<(), _> {
                host.push("read:bad");
                pending.request(&record("after-error", None)).unwrap();
                Err(ViewException::new("bad read"))
            },
            |(), host, _| {
                host.push("write:bad");
                Ok(())
            },
        ),
        MeasureRequest::read_only(None, |host: &Host, _| {
            host.push("read:read-only");
            Ok(())
        }),
        MeasureRequest::new(
            None,
            |host: &Host, _| {
                host.push("read:bad-write");
                Ok("result")
            },
            |result, host, _| {
                host.push(format!("write:bad-write:{result}"));
                Err(ViewException::new("bad write"))
            },
        ),
        record("survivor", None),
    ] {
        queue.requests().request(&request).unwrap();
    }
    assert_eq!(drain(&mut queue, &mut host).rounds, 2);
    assert_eq!(host.events(), upstream()["errors"]["events"]);
}

#[test]
fn self_rescheduling_leaves_a_backlog_after_six_rounds() {
    let mut queue = MeasureQueue::new();
    let mut host = Host::default();
    let request = MeasureRequest::new(
        None,
        |host: &Host, _| {
            host.push(format!("read:{}", host.value));
            Ok(host.value)
        },
        |result, host, pending| {
            host.push(format!("write:{result}"));
            host.value += 1;
            if host.value < 8 {
                pending.request(host.repeat.as_ref().unwrap()).unwrap();
            }
            Ok(())
        },
    );
    host.repeat = Some(request.clone());
    queue.requests().request(&request).unwrap();
    let expected = &upstream()["roundLimit"];
    assert_eq!(
        drain(&mut queue, &mut host),
        MeasureDrain {
            rounds: MAX_MEASURE_ROUNDS,
            pending: true
        }
    );
    assert_eq!(host.value, expected["afterFirst"]["count"]);
    assert_eq!(
        drain(&mut queue, &mut host),
        MeasureDrain {
            rounds: 2,
            pending: false
        }
    );
    assert_eq!(host.value, expected["afterSecond"]["count"]);
    assert_eq!(host.events(), expected["events"]);
}

#[test]
fn unstable_layout_defers_the_pending_batch_until_the_host_drains_again() {
    let mut queue = MeasureQueue::new();
    let mut host = Host::default();
    queue.requests().request(&record("deferred", None)).unwrap();
    let first = queue.drain(
        &mut host,
        |host| {
            host.push("layout:changed");
            false
        },
        report,
    );
    assert_eq!(
        first,
        MeasureDrain {
            rounds: 0,
            pending: true
        }
    );
    let second = queue.drain(
        &mut host,
        |host| {
            host.push("layout:stable");
            true
        },
        report,
    );
    assert_eq!(
        second,
        MeasureDrain {
            rounds: 1,
            pending: false
        }
    );
    assert_eq!(host.events(), upstream()["viewport"]["events"]);
}

#[test]
fn pending_read_uses_current_host_and_the_queue_does_not_outlive_its_owner() {
    let mut queue = MeasureQueue::new();
    let mut host = Host {
        doc: "a",
        ..Host::default()
    };
    queue
        .requests()
        .request(&MeasureRequest::read_only(None, |host: &Host, _| {
            host.push(format!("read:{}", host.doc));
            Ok(())
        }))
        .unwrap();
    host.doc = "b";
    assert_eq!(drain(&mut queue, &mut host).rounds, 1);
    queue
        .requests()
        .request(&record("destroyed", None))
        .unwrap();
    drop(queue);
    assert_eq!(host.events(), upstream()["staleAndDestroyed"]["events"]);
}

#[test]
fn can_discard_pending_work_without_changing_the_active_round() {
    let mut queue = MeasureQueue::new();
    let mut host = Host::default();
    queue
        .requests()
        .request(&MeasureRequest::read_only(None, |_, pending| {
            pending.request(&record("stale", Some(1))).unwrap();
            pending.request(&record("also-stale", None)).unwrap();
            assert!(pending.cancel(1));
            assert!(!pending.cancel(1));
            pending.clear();
            Ok(())
        }))
        .unwrap();
    queue
        .requests()
        .request(&record("active", Some(1)))
        .unwrap();
    assert_eq!(
        drain(&mut queue, &mut host),
        MeasureDrain {
            rounds: 1,
            pending: false
        }
    );
    assert_eq!(host.events(), json!(["read:active", "write:active"]));
}

#[test]
fn full_queue_accepts_duplicates_and_replacements_but_not_new_keys() {
    let mut queue = MeasureQueue::new();
    let mut host = Host::default();
    let first = record("original", Some(0));
    let pending = queue.requests();
    pending.request(&first).unwrap();
    for key in 1..MAX_PENDING_MEASURES as MeasureKey {
        pending.request(&record("other", Some(key))).unwrap();
    }
    pending.request(&first).unwrap();
    pending.request(&record("replacement", Some(0))).unwrap();
    assert_eq!(
        pending.request(&record("overflow", None)),
        Err(MeasureQueueFull)
    );
    assert_eq!(
        drain(&mut queue, &mut host),
        MeasureDrain {
            rounds: 1,
            pending: false
        }
    );
    let events = host.events.borrow();
    assert_eq!(events.len(), 2 * MAX_PENDING_MEASURES);
    assert_eq!(events[0], "read:replacement");
    assert_eq!(events[MAX_PENDING_MEASURES], "write:replacement");
    assert!(
        events[1..MAX_PENDING_MEASURES]
            .iter()
            .all(|event| event == "read:other")
    );
    assert!(
        events[MAX_PENDING_MEASURES + 1..]
            .iter()
            .all(|event| event == "write:other")
    );
}
