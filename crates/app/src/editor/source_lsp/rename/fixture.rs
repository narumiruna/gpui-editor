//! Std-only framed peer for the live rename tests.

use std::{
    fs,
    io::{self, BufRead, Write},
    panic,
};

const URI: &str = "file:///rename-draft.md";
const SOURCE: &str = "Lead\r\n\r\n```rs\r\n// 😀foo + foo\r\n```\r\n\r\n```rs\r\nfoo\r\n```\r\n";

fn read(input: &mut impl BufRead) -> String {
    let mut header = String::new();
    input.read_line(&mut header).unwrap();
    let size = header
        .strip_prefix("Content-Length: ")
        .unwrap_or_else(|| panic!("missing Content-Length: {header:?}"))
        .trim_end()
        .parse()
        .unwrap();
    header.clear();
    input.read_line(&mut header).unwrap();
    assert_eq!(header, "\r\n");
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn has(frame: &str, part: &str) {
    assert!(frame.contains(part), "expected {part} in {frame}");
}

fn method(frame: &str, name: &str) {
    has(frame, &format!(r#""method":"{name}""#));
}

fn id(frame: &str) -> i32 {
    frame
        .split_once(r#""id":"#)
        .unwrap()
        .1
        .split([',', '}'])
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn send(output: &mut impl Write, body: &str) {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}

fn result(output: &mut impl Write, id: i32, value: &str) {
    send(
        output,
        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{value}}}"#),
    );
}

fn rename(frame: &str, name: &str) -> i32 {
    method(frame, "textDocument/rename");
    has(
        frame,
        &format!(
            r#""params":{{"textDocument":{{"uri":"{URI}"}},"position":{{"line":3,"character":5}},"newName":{name:?}}}"#
        ),
    );
    id(frame)
}

fn edit(start: (u32, u32), end: (u32, u32), text: &str) -> String {
    let pos = |(line, character)| format!(r#"{{"line":{line},"character":{character}}}"#);
    format!(
        r#"{{"range":{{"start":{},"end":{}}},"newText":{text:?}}}"#,
        pos(start),
        pos(end)
    )
}

fn changes(edits: &str) -> String {
    format!(r#"{{"changes":{{"{URI}":[{edits}]}}}}"#)
}

fn batch(version: &str, edits: &str) -> String {
    format!(r#"{{"textDocument":{{"uri":"{URI}","version":{version}}},"edits":[{edits}]}}"#)
}

fn first(text: &str) -> String {
    edit((3, 5), (3, 8), text)
}

fn both(text: &str) -> String {
    format!("{},{}", edit((3, 11), (3, 14), text), first(text))
}

fn did_change(input: &mut impl BufRead, version: u32, expected: &str) {
    let frame = read(input);
    method(&frame, "textDocument/didChange");
    has(&frame, &format!(r#""version":{version}"#));
    has(
        &frame,
        &format!(r#""contentChanges":[{{"text":{expected:?}}}]"#),
    );
}

fn flow(input: &mut impl BufRead, output: &mut impl Write) {
    let synced = SOURCE.replacen("Lead", "Changed", 1);
    did_change(input, 1, &synced);
    let request = rename(&read(input), "élan");
    let valid = changes(&both("élan"));
    result(output, request + 500, &valid);
    send(
        output,
        &format!(
            r#"{{"jsonrpc":"2.0","id":"peer-edit","method":"workspace/applyEdit","params":{{"edit":{valid}}}}}"#
        ),
    );
    let reply = read(input);
    has(&reply, r#""id":"peer-edit""#);
    has(&reply, r#""code":-32601"#);
    result(output, request, &valid);
    did_change(input, 2, &synced.replacen("foo + foo", "élan + élan", 1));
    did_change(input, 3, &synced);
    did_change(input, 4, &synced.replacen("foo + foo", "élan + élan", 1));
}

fn bad_results(input: &mut impl BufRead, output: &mut impl Write) {
    let valid = first("bar");
    let same = batch("0", &valid);
    let stale = batch("1", &valid);
    let foreign = "https://example.invalid/do-not-follow";
    let replies = [
        "null".into(),
        "{}".into(),
        changes(""),
        changes(&first("foo")),
        format!(r#"{{"changes":{{"{URI}":[{valid}],"{foreign}":[]}}}}"#),
        format!(r#"{{"changes":{{"{foreign}":[{valid}]}},"documentChanges":[{same}]}}"#),
        format!(
            r#"{{"documentChanges":[{same},{{"kind":"rename","oldUri":"{URI}","newUri":"{foreign}"}}]}}"#
        ),
        format!(r#"{{"documentChanges":[{same},{same}]}}"#),
        format!(r#"{{"documentChanges":[{stale}]}}"#),
        changes(&format!(
            r#"{valid},{{"annotationId":"ask",{}}}"#,
            &valid[1..valid.len() - 1]
        )),
        changes(&format!("{valid},{valid}")),
        changes(&format!("{valid},{}", edit((3, 4), (3, 5), "!"))), // Split surrogate.
        changes(&format!("{valid},{}", edit((0, 0), (0, 0), ""))),  // Outside-body no-op.
        changes(&format!("{valid},{}", edit((2, 0), (2, 3), "!!!"))),
        changes(&format!("{valid},{}", edit((3, 14), (4, 0), ""))), // Closing separator.
        changes(&format!("{valid},{}", edit((3, 14), (3, 14), "\r"))), // Joins protected LF.
        changes(&format!("{valid},{}", edit((7, 0), (7, 3), "bar"))),
        // Neither edit alone closes the fence, but they do when applied together.
        changes(&format!(
            "{},{}",
            edit((3, 0), (3, 5), "`"),
            edit((3, 5), (3, 14), "``")
        )),
        changes(&vec![valid.as_str(); 257].join(",")),
        changes(&format!("{valid},{{}}")),
    ];
    for value in replies {
        result(output, rename(&read(input), "bar"), &value);
    }
    // Same-offset insertions must retain their original order before replacement.
    let insert_a = edit((3, 5), (3, 5), "a");
    let insert_b = edit((3, 5), (3, 5), "b");
    let edits = format!("{insert_a},{insert_b},{}", both("bar"));
    result(
        output,
        rename(&read(input), "bar"),
        &format!(r#"{{"documentChanges":[{}]}}"#, batch("0", &edits)),
    );
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().unwrap().into_string().unwrap();
    let log = args.next().unwrap();
    panic::set_hook(Box::new(move |panic| {
        let _ = fs::write(&log, panic.to_string());
    }));
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let initialize = read(&mut input);
    method(&initialize, "initialize");
    let capabilities = match mode.as_str() {
        "unsupported" => r#"{"capabilities":{"textDocumentSync":1,"renameProvider":false}}"#,
        "no-sync" => r#"{"capabilities":{"renameProvider":true}}"#,
        _ => r#"{"capabilities":{"textDocumentSync":1,"renameProvider":{"prepareProvider":true}}}"#,
    };
    result(&mut output, id(&initialize), capabilities);
    method(&read(&mut input), "initialized");
    let open = read(&mut input);
    method(&open, "textDocument/didOpen");
    let source = if mode == "bad-results" {
        SOURCE.replace("foo + foo\r\n", "foo + foo\n")
    } else {
        SOURCE.into()
    };
    has(&open, &format!(r#""text":{source:?}"#));
    has(&open, &format!(r#""uri":"{URI}""#));
    has(&open, r#""version":0"#);
    let close = match mode.as_str() {
        "flow" | "bad-results" | "no-sync" => {
            match mode.as_str() {
                "flow" => flow(&mut input, &mut output),
                "bad-results" => bad_results(&mut input, &mut output),
                _ => result(
                    &mut output,
                    rename(&read(&mut input), "bar"),
                    &changes(&both("bar")),
                ),
            }
            read(&mut input)
        }
        "unsupported" => read(&mut input),
        "repeat" => loop {
            let frame = read(&mut input);
            if frame.contains(r#""method":"textDocument/didClose""#) {
                break frame;
            }
            if !frame.contains(r#""method":"textDocument/didChange""#) {
                result(&mut output, rename(&frame, "bar"), &changes(&both("bar")));
            }
        },
        _ => panic!("unknown fixture mode {mode}"),
    };
    method(&close, "textDocument/didClose");
    has(&close, URI);
    let shutdown = read(&mut input);
    method(&shutdown, "shutdown");
    result(&mut output, id(&shutdown), "null");
    method(&read(&mut input), "exit");
}
