//! `bullscript lsp` — a language server for `.busc` files.
//!
//! What it does is deliberately narrow: it tells you where a pipe is wrong,
//! while you type. A missing semicolon, an unknown builtin, a binding whose
//! declared type does not match what the pipe produces — each becomes a red
//! underline on the line that caused it.
//!
//! It can do that because BullScript already lexes, parses and type checks
//! every program before running a single pipe. The server adds no analysis of
//! its own; it runs the same three passes the interpreter does and reports
//! what they say. That is the point: the squiggle in the editor and the error
//! at the prompt can never disagree, because they come from the same code.
//!
//! Structure follows `bullarchy`'s server, including two things learned there:
//!
//!   - **Diagnostics come from the buffer, not the file.** `didChange` is the
//!     only thing that knows what you are looking at; validating the saved
//!     file means reporting errors you have already fixed.
//!   - **Debounced.** Reading runs on its own thread so the main loop can wait
//!     with a timeout. Checking on every keystroke is both wasteful and wrong:
//!     mid-word, every line is missing its semicolon.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use crate::lang;

/// How long the buffer must be quiet before it is re-checked.
const DEBOUNCE: Duration = Duration::from_millis(300);

pub fn run() {
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut shutdown_requested = false;

    let (tx, rx) = mpsc::channel::<Msg>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = io::BufReader::new(stdin.lock());
        while let Some(msg) = read_message(&mut reader) {
            if tx.send(msg).is_err() {
                break;
            }
        }
    });

    let mut pending: Option<String> = None;

    loop {
        let timeout = if pending.is_some() {
            DEBOUNCE
        } else {
            Duration::from_secs(3600)
        };
        match rx.recv_timeout(timeout) {
            Ok(msg) => {
                if let Some(uri) = handle(msg, &mut docs, &mut writer, &mut shutdown_requested) {
                    pending = Some(uri);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(uri) = pending.take() {
                    publish(&uri, docs.get(&uri).map(String::as_str).unwrap_or(""), &mut writer);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

// ── Protocol ──────────────────────────────────────────────────────────────

/// A parsed request or notification: method, id, params.
struct Msg {
    method: String,
    id: Option<String>,
    params: String,
}

fn read_message(reader: &mut impl BufRead) -> Option<Msg> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        // Header names are case-insensitive in the base protocol, and the
        // space after the colon is optional.
        if let Some((name, rest)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = rest.trim().parse().ok();
            }
        }
    }

    let len = content_length?;
    let mut buf = vec![0u8; len];
    std::io::Read::read_exact(reader, &mut buf).ok()?;
    let body = String::from_utf8(buf).ok()?;

    Some(Msg {
        method: json_str(&body, "method").unwrap_or_default(),
        id: json_raw(&body, "id"),
        params: body,
    })
}

fn send(writer: &mut impl Write, body: &str) {
    let _ = write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = writer.flush();
}

fn handle(
    msg: Msg,
    docs: &mut HashMap<String, String>,
    writer: &mut impl Write,
    shutdown_requested: &mut bool,
) -> Option<String> {
    match msg.method.as_str() {
        "initialize" => {
            let id = msg.id.unwrap_or_else(|| "null".to_string());
            send(
                writer,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"capabilities":{{"textDocumentSync":1}},"serverInfo":{{"name":"bullscript","version":"{}"}}}}}}"#,
                    env!("CARGO_PKG_VERSION")
                ),
            );
        }
        "shutdown" => {
            *shutdown_requested = true;
            let id = msg.id.unwrap_or_else(|| "null".to_string());
            send(writer, &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":null}}"#));
        }
        // Per the spec, `exit` without a preceding `shutdown` is an
        // unexpected end and should be distinguishable by exit status.
        "exit" => std::process::exit(if *shutdown_requested { 0 } else { 1 }),

        "textDocument/didOpen" | "textDocument/didChange" | "textDocument/didSave" => {
            let uri = json_str(&msg.params, "uri")?;
            if let Some(text) = json_str(&msg.params, "text") {
                docs.insert(uri.clone(), text);
            }
            return Some(uri);
        }
        "textDocument/didClose" => {
            let uri = json_str(&msg.params, "uri")?;
            docs.remove(&uri);
            // Clear this file's diagnostics, or the editor keeps showing the
            // last ones for a buffer that is no longer open.
            send(writer, &diagnostics_notification(&uri, ""));
        }
        _ => {
            // Requests need an answer even when unsupported; notifications do
            // not, and answering one is a protocol error.
            if let Some(id) = msg.id {
                send(
                    writer,
                    &format!(
                        r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"method not found"}}}}"#
                    ),
                );
            }
        }
    }
    None
}

// ── Diagnostics ───────────────────────────────────────────────────────────

fn publish(uri: &str, source: &str, writer: &mut impl Write) {
    send(writer, &diagnostics_notification(uri, &check(source, uri)));
}

fn diagnostics_notification(uri: &str, items: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{}","diagnostics":[{}]}}}}"#,
        json_escape(uri),
        items
    )
}

/// Run the same three passes the interpreter runs, and report the first thing
/// that fails.
///
/// One diagnostic at a time, because that is genuinely all there is: the lexer
/// and parser stop at the first error rather than recovering, so claiming to
/// report more would mean inventing them.
fn check(source: &str, uri: &str) -> String {
    // A bag entry's signature is needed to type a `bag::` call. Resolving it
    // has to work here exactly as it does at the prompt, or a script that runs
    // would show errors in the editor.
    let err = match lang::parse_source(source) {
        Err(e) => Some(e),
        Ok(program) => {
            lang::check::check_program(&program, &HashMap::new(), &|n| crate::bag::signature(n))
                .err()
        }
    };

    let Some(err) = err else { return String::new() };

    // BsError carries a line but no column, so the diagnostic covers the whole
    // line. For "expected ';'" that is the right shape anyway — the mistake is
    // the line, not one character in it.
    // "found end of input" is reported at the line *after* the last one, which
    // is past the end of the buffer — an editor draws that nowhere useful. The
    // mistake is on the last line that has text on it.
    let lines: Vec<&str> = source.lines().collect();
    let last = lines.len().saturating_sub(1);
    let mut line_no = err.line.unwrap_or(1).max(1) - 1;
    if line_no > last {
        line_no = last;
    }
    while line_no > 0 && lines.get(line_no).is_none_or(|l| l.trim().is_empty()) {
        line_no -= 1;
    }
    let line_text = lines.get(line_no).copied().unwrap_or("");
    let start = line_text.len() - line_text.trim_start().len();
    let end = line_text.trim_end().chars().count().max(start + 1);

    let _ = uri;
    format!(
        r#"{{"range":{{"start":{{"line":{line_no},"character":{start}}},"end":{{"line":{line_no},"character":{end}}}}},"severity":1,"source":"bullscript","message":"{}"}}"#,
        json_escape(&err.message)
    )
}

// ── Minimal JSON reading ──────────────────────────────────────────────────
//
// Enough to pull three fields out of a request. BullScript has no JSON
// dependency and this does not justify adding one: the server reads `method`,
// `id`, `uri` and `text`, and writes messages it builds itself.

/// The string value of `"key": "..."`, unescaped.
fn json_str(body: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":");
    let mut from = 0;
    // `text` appears inside contentChanges as well as at the top level; the
    // last occurrence is the most recent content.
    let mut found = None;
    while let Some(i) = body[from..].find(&pat) {
        let start = from + i + pat.len();
        let rest = body[start..].trim_start();
        if rest.starts_with('"') {
            found = Some(start + (body[start..].len() - rest.len()));
        }
        from = start;
    }
    let quote = found?;
    let bytes = body.as_bytes();
    let mut i = quote + 1;
    let mut out = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                i += 1;
                out.push(match bytes[i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'u' => {
                        let hex = body.get(i + 1..i + 5)?;
                        let cp = u32::from_str_radix(hex, 16).ok()?;
                        i += 4;
                        char::from_u32(cp).unwrap_or('\u{fffd}')
                    }
                    other => other as char,
                });
            }
            b'"' => return Some(out),
            _ => {
                // Multi-byte UTF-8 has to be copied whole.
                let ch = body[i..].chars().next()?;
                out.push(ch);
                i += ch.len_utf8() - 1;
            }
        }
        i += 1;
    }
    None
}

/// The raw value of `"key": <value>` — used for `id`, which may be a number or
/// a string and must be echoed back in whichever form it arrived.
fn json_raw(body: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":");
    let start = body.find(&pat)? + pat.len();
    let rest = body[start..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        return Some(format!("\"{}\"", &stripped[..end]));
    }
    let end = rest.find([',', '}'])?;
    let value = rest[..end].trim();
    if value == "null" || value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
