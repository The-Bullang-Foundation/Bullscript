//! The fixed `builtin::*` table. Prototypes are fixed at BullScript's own
//! build time; these are never stored in bag.json and can't be removed via
//! `bag::remove`.
//!
//! Takes inspiration from Bullang's stdlib naming (`to_upper`, `to_lower`,
//! `trim`, `out`, `open`, `close`, `in`) for familiarity, without reusing
//! Bullang's code.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::process::Command;
use std::sync::Mutex;

use super::error::BsError;
use super::types::{BsType, Value};

// ── File descriptor table ─────────────────────────────────────────────────
//
// `open` allocates from 3 upward. Descriptors 0/1/2 are never in this table:
// they are the process's own stdin/stdout/stderr and are handled directly.
//
// A descriptor that is not in this table is not automatically an error — the
// parent process may have handed us an inherited descriptor. In that case we
// fall through to the raw OS descriptor and let the OS report failure, which
// `out`/`in`/`close` surface through their return value or an error.

struct FdTable {
    next:  i64,
    files: HashMap<i64, File>,
}

impl FdTable {
    fn new() -> Self {
        FdTable { next: 3, files: HashMap::new() }
    }
}

static FD_TABLE: Mutex<Option<FdTable>> = Mutex::new(None);

fn with_fds<F, T>(f: F) -> T
where F: FnOnce(&mut FdTable) -> T {
    let mut guard = FD_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(FdTable::new());
    }
    f(guard.as_mut().unwrap())
}

/// Close every descriptor opened by `builtin::open`. Called on normal exit so
/// buffered writes reach disk even when a script forgets to `close`.
pub fn close_all_fds() {
    with_fds(|t| t.files.clear());
}

/// Returns the declared prototype for a builtin, or None if it doesn't exist.
pub fn prototype(name: &str) -> Option<(Vec<BsType>, BsType)> {
    match name {
        "add"      => Some((vec![BsType::I64, BsType::I64], BsType::I64)),
        "to_upper" => Some((vec![BsType::String], BsType::String)),
        "to_lower" => Some((vec![BsType::String], BsType::String)),
        "trim"     => Some((vec![BsType::String], BsType::String)),
        "i64_to_str" => Some((vec![BsType::I64], BsType::String)),
        "str_to_i64" => Some((vec![BsType::String], BsType::I64)),
        "out"      => Some((vec![BsType::I64, BsType::String], BsType::Bool)),
        "in"       => Some((vec![BsType::I64], BsType::String)),
        "open"     => Some((vec![BsType::String, BsType::String], BsType::I64)),
        "close"    => Some((vec![BsType::I64], BsType::Bool)),
        "run"      => Some((vec![BsType::String], BsType::Bool)),
        "capture"  => Some((vec![BsType::String], BsType::String)),
        _          => None,
    }
}

/// Every builtin name, for help text and diagnostics.
pub const NAMES: &[&str] = &[
    "add", "to_upper", "to_lower", "trim", "i64_to_str", "str_to_i64",
    "out", "in", "open", "close",
    "run", "capture",
];

pub fn call(name: &str, args: &[Value]) -> Result<Value, BsError> {
    match name {
        "add" => {
            let (a, b) = (as_i64(args, 0)?, as_i64(args, 1)?);
            a.checked_add(b)
                .map(Value::I64)
                .ok_or_else(|| BsError::new(format!(
                    "integer overflow: {} + {} does not fit in an i64", a, b
                )))
        }
        "to_upper" => Ok(Value::Str(as_str(args, 0)?.to_uppercase())),
        "to_lower" => Ok(Value::Str(as_str(args, 0)?.to_lowercase())),
        "trim"     => Ok(Value::Str(as_str(args, 0)?.trim().to_string())),

        // builtin::i64_to_str(x: i64) -> String
        "i64_to_str" => Ok(Value::Str(as_i64(args, 0)?.to_string())),

        // builtin::str_to_i64(s: String) -> i64
        //
        // **Zero when the string does not parse.** Not an error, and not a
        // value the caller has to test for. That is what Bullang's builtin of
        // the same name does on all six of its backends, and the two languages
        // sharing a name while disagreeing on what it means would be worse
        // than either choice on its own.
        //
        // Surrounding whitespace is ignored, so a line read with builtin::in
        // converts without a trim first.
        "str_to_i64" => Ok(Value::I64(
            as_str(args, 0)?.trim().parse::<i64>().unwrap_or(0)
        )),

        // builtin::out(fd: i64, content: String) -> bool
        //
        // Writes `content` verbatim — no newline is appended. Supply '\n'
        // yourself when you want one.
        //
        // 1 and 2 are the process's own stdout/stderr. 3+ is either a
        // descriptor from builtin::open or one inherited from the parent
        // process. The bool reports whether the write succeeded.
        "out" => {
            let fd  = as_i64(args, 0)?;
            let msg = as_str(args, 1)?.to_string();
            Ok(Value::Bool(write_fd(fd, msg.as_bytes())))
        }

        // builtin::in(fd: i64) -> String
        //
        // Reads one line, without its trailing newline. Returns an empty
        // string at end of input.
        "in" => {
            let fd = as_i64(args, 0)?;
            read_line_fd(fd).map(Value::Str)
        }

        // builtin::open(path: String, mode: String) -> i64
        //
        // Modes: r, w, a, rw. Returns the new descriptor. Failure to open is
        // a hard error rather than a sentinel: BullScript has no conditionals,
        // so a script could not branch on a -1 anyway and would just pass a
        // bad descriptor onward and fail later with a worse message.
        "open" => {
            let path = as_str(args, 0)?.to_string();
            let mode = as_str(args, 1)?.to_string();
            let file = match mode.as_str() {
                "r"  => OpenOptions::new().read(true).open(&path),
                "w"  => OpenOptions::new().write(true).create(true).truncate(true).open(&path),
                "a"  => OpenOptions::new().append(true).create(true).open(&path),
                "rw" => OpenOptions::new().read(true).write(true).create(true).open(&path),
                other => return Err(BsError::new(format!(
                    "open: unknown mode '{}' — use 'r', 'w', 'a' or 'rw'", other
                ))),
            }.map_err(|e| BsError::new(format!("open: could not open '{}': {}", path, e)))?;

            Ok(Value::I64(with_fds(|t| {
                let fd = t.next;
                t.next += 1;
                t.files.insert(fd, file);
                fd
            })))
        }

        // builtin::close(fd: i64) -> bool
        //
        // False if the descriptor was not open. Closing 0/1/2 is an error:
        // they belong to the process, not to the script.
        "close" => {
            let fd = as_i64(args, 0)?;
            if (0..=2).contains(&fd) {
                return Err(BsError::new(format!(
                    "close: refusing to close fd {} — 0, 1 and 2 are the process's own \
                     standard streams", fd
                )));
            }
            Ok(Value::Bool(with_fds(|t| t.files.remove(&fd).is_some())))
        }

        // builtin::run(cmd: String) -> bool — success/failure only, output discarded.
        "run" => {
            let cmd = as_str(args, 0)?;
            let status = shell_inherit(cmd)?;
            // The child owns the terminal while it runs, so where it left the
            // cursor is not something we can see. Assume the worst: the REPL
            // will finish the line before its next prompt, which costs a blank
            // line after a command that ended tidily and saves a line of
            // output from a command that did not.
            mark_terminal_dirty();
            Ok(Value::Bool(status.success()))
        }

        // builtin::capture(cmd: String) -> String — stdout only, no status info.
        // Split from `run` because the type pool has no Tuple: one call can
        // only bind one typed value.
        "capture" => {
            let cmd = as_str(args, 0)?;
            let output = shell(cmd)?;
            Ok(Value::Str(String::from_utf8_lossy(&output.stdout).into_owned()))
        }

        _ => Err(BsError::new(format!("unknown builtin 'builtin::{}'", name))),
    }
}

// ── fd helpers ────────────────────────────────────────────────────────────

/// Whether the last thing written to the terminal ended with a newline.
///
/// `builtin::out` writes exactly what it is given and appends nothing, so a
/// pipe can leave the cursor part-way along a line. The REPL needs to know,
/// because rustyline draws its prompt with a carriage return followed by
/// erase-to-end-of-line — which wipes whatever is on that line. Short output
/// vanished entirely; output long enough to wrap lost only its last row, which
/// looked like truncation.
static AT_LINE_START: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// True if the terminal cursor is at the start of a line.
pub fn at_line_start() -> bool {
    AT_LINE_START.load(std::sync::atomic::Ordering::Relaxed)
}

/// Note that something we cannot see has written to the terminal — a child
/// process that inherited it. Where it left the cursor is unknowable, so the
/// REPL assumes the line is unfinished.
pub fn mark_terminal_dirty() {
    AT_LINE_START.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Note that the cursor has been returned to the start of a line by something
/// other than a write — the REPL calls this after emitting its own newline.
pub fn mark_line_start() {
    AT_LINE_START.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn write_fd(fd: i64, bytes: &[u8]) -> bool {
    // Only the two terminal streams move the cursor the REPL cares about. A
    // write to an opened file does not.
    if (fd == 1 || fd == 2) && !bytes.is_empty() {
        AT_LINE_START.store(
            bytes.last() == Some(&b'\n'),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    match fd {
        1 => {
            let stdout = std::io::stdout();
            let mut h = stdout.lock();
            h.write_all(bytes).and_then(|_| h.flush()).is_ok()
        }
        2 => {
            let stderr = std::io::stderr();
            let mut h = stderr.lock();
            h.write_all(bytes).and_then(|_| h.flush()).is_ok()
        }
        fd => {
            let opened = with_fds(|t| match t.files.get_mut(&fd) {
                Some(f) => Some(f.write_all(bytes).and_then(|_| f.flush()).is_ok()),
                None    => None,
            });
            match opened {
                Some(ok) => ok,
                // Not ours — try the raw descriptor, which the parent process
                // may have handed down.
                None => write_raw_fd(fd, bytes),
            }
        }
    }
}

fn read_line_fd(fd: i64) -> Result<String, BsError> {
    if fd == 0 {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)
            .map_err(|e| BsError::new(format!("in: could not read stdin: {}", e)))?;
        return Ok(strip_newline(line));
    }
    if fd == 1 || fd == 2 {
        return Err(BsError::new(format!(
            "in: fd {} is an output stream and cannot be read", fd
        )));
    }

    // Read byte by byte so a single `in` consumes exactly its own line and
    // leaves the rest of the file for the next call.
    let from_table = with_fds(|t| {
        t.files.get_mut(&fd).map(|f| read_line_from(f))
    });
    match from_table {
        Some(r) => r,
        None    => read_line_raw_fd(fd),
    }
}

fn read_line_from(r: &mut impl Read) -> Result<String, BsError> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match r.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' { break; }
                line.push(byte[0]);
            }
            Err(e) => return Err(BsError::new(format!("in: read failed: {}", e))),
        }
    }
    Ok(strip_newline(String::from_utf8_lossy(&line).into_owned()))
}

fn strip_newline(mut s: String) -> String {
    if s.ends_with('\n') { s.pop(); }
    if s.ends_with('\r') { s.pop(); }
    s
}

#[cfg(unix)]
fn write_raw_fd(fd: i64, bytes: &[u8]) -> bool {
    use std::os::fd::FromRawFd;
    if fd < 0 || fd > i32::MAX as i64 { return false; }
    let mut f = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(fd as i32) });
    f.write_all(bytes).and_then(|_| f.flush()).is_ok()
}

#[cfg(not(unix))]
fn write_raw_fd(_fd: i64, _bytes: &[u8]) -> bool {
    false
}

#[cfg(unix)]
fn read_line_raw_fd(fd: i64) -> Result<String, BsError> {
    use std::os::fd::FromRawFd;
    if fd < 0 || fd > i32::MAX as i64 {
        return Err(BsError::new(format!("in: fd {} is not a valid descriptor", fd)));
    }
    let mut f = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(fd as i32) });
    read_line_from(&mut *f)
}

#[cfg(not(unix))]
fn read_line_raw_fd(fd: i64) -> Result<String, BsError> {
    Err(BsError::new(format!(
        "in: fd {} is not open — inherited descriptors are not supported on this platform", fd
    )))
}

// ── shell ─────────────────────────────────────────────────────────────────

/// Run a command with its output captured — for `builtin::capture`, which
/// hands that output back as a String.
fn shell(cmd: &str) -> Result<std::process::Output, BsError> {
    let (program, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    Command::new(program)
        .arg(flag)
        .arg(cmd)
        .output()
        .map_err(|e| BsError::new(format!("failed to run command: {}", e)))
}

/// Run a command with the terminal handed straight to it — for `builtin::run`,
/// which is about making something happen rather than reading its output.
///
/// This used `.output()`, which captures both streams and then throws them
/// away: `builtin::run("echo hi")` printed nothing, and a command that wanted
/// to ask a question got an empty stdin and hung or failed. `run` and
/// `capture` were the same call, differing only in what they returned.
///
/// `.status()` inherits all three streams, so the child prints where the user
/// can see it and can prompt for input. Use `capture` when the output is what
/// you are after.
fn shell_inherit(cmd: &str) -> Result<std::process::ExitStatus, BsError> {
    let (program, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    Command::new(program)
        .arg(flag)
        .arg(cmd)
        .status()
        .map_err(|e| BsError::new(format!("failed to run command: {}", e)))
}

// ── argument accessors ────────────────────────────────────────────────────
//
// These take the whole slice and an index rather than a single Value so a
// caller that reaches `call` without going through prototype validation gets
// an error instead of an out-of-bounds panic.

fn as_i64(args: &[Value], i: usize) -> Result<i64, BsError> {
    match args.get(i) {
        Some(Value::I64(n)) => Ok(*n),
        Some(other) => Err(BsError::new(format!(
            "argument {} expected i64, found {}", i + 1, other.ty()
        ))),
        None => Err(BsError::new(format!("missing argument {}", i + 1))),
    }
}

fn as_str(args: &[Value], i: usize) -> Result<&str, BsError> {
    match args.get(i) {
        Some(Value::Str(s)) => Ok(s),
        Some(other) => Err(BsError::new(format!(
            "argument {} expected String, found {}", i + 1, other.ty()
        ))),
        None => Err(BsError::new(format!("missing argument {}", i + 1))),
    }
}
