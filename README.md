# BullScript

BullScript is a small, pipe-only interpreted language for the
[Bullang](https://github.com/The-Bullang-Foundation/Bullang) ecosystem. It
borrows Bullang's pipe syntax for familiarity but is its own grammar and
evaluator — Bullang itself is never modified or extended by BullScript, and
BullScript does not depend on the Bullang crate.

`bullscript` is the interpreter itself. There is no command dispatcher.

---

## Prerequisite

Cargo v1.92.0 or later (edition 2024).

## Installation

```bash
cargo install --git https://github.com/The-Bullang-Foundation/Bullscript.git
```

Reinstalling over an existing version:

```bash
cargo install --git https://github.com/The-Bullang-Foundation/Bullscript.git --force bullscript
```

---

## Usage

```bash
bullscript
```

Drops straight into the interactive prompt. This *is* the whole program.

```bash
bullscript path/to/script.busc [arguments...]
```

Runs a `.busc` file non-interactively. Arguments are parsed into the types
the script's first pipe declares; a mismatch is reported before anything
runs. When the script finishes, its return value is printed.

```bash
bullscript --help
bullscript --version
```

---

## The language

A BullScript program — whether a `.busc` file or a line typed at the
prompt — is nothing but a sequence of pipes:

```
( <input>: <type>, ... ) : <callee-or-expr> -> { <name>: <type> } ;
```

- **Every input and every created binding always carries an explicit
  type.** There's no inference and no `let` — this is deliberate, for
  readability and to catch mismatches early.
- The middle section is either a call (`builtin::name` or `bag::name`,
  taking the pipe's inputs as its arguments, in order) or a bare
  arithmetic/comparison/logical expression over the pipe's own inputs
  (`+ - * /`, `== != < > <= >=`, `&& ||`, unary `-`/`!`, parens).
- `-> {}` discards the result. `-> {name: type}` creates or overwrites
  `name` with the computed value.
- Exactly four types: `i64`, `f64`, `bool`, `String`. No tuples, no
  arrays, no other widths — this is what lets a bare literal be
  unambiguous without a prototype in scope.

```
(a: i64, b: i64) : builtin::add           -> {sum: i64};
(sum: i64)       : sum * 2                -> {doubled: i64};
(1: i64, "done\n": String) : builtin::out -> {ok: bool};
```

### Everything is checked before anything runs

A program is parsed and fully type checked before its first pipe executes.
A mistake anywhere in the file is reported without any of the earlier pipes
having taken effect — no half-written files, no shell commands already run.

```
(1: i64, "starting\n": String) : builtin::out -> {r: bool};
(5: i64) : builtin::to_upper -> {z: String};
```

This prints nothing. It fails with *`builtin 'to_upper' argument 1 expects
String, got i64`* before the first pipe runs.

### Literals

`i64` (`42`), `f64` (`3.5`), `String` (`"text"`), and `bool` (`true` /
`false`). `true` and `false` are reserved words and cannot be used as
binding names.

String escapes: `\n` `\t` `\r` `\0` `\"` `\\`. A string literal cannot span
lines — use `\n`.

### Operators

Standard precedence: arithmetic binds tighter than comparison, which binds
tighter than logical. (Bullang's own pipes chain flat, left to right;
BullScript deliberately differs here.)

`&&` and `||` short-circuit, so a guard like `(n != 0) && (100 / n > 5)`
is safe.

Arithmetic requires two `i64` or two `f64` — never a mix. So do `==` and
`!=`: comparing across types could never be true, so it is rejected rather
than silently answered `false`.

Integer overflow is an error, not a wrap and not a crash.

### File descriptors

`1` is stdout and `2` is stderr. `builtin::open` returns `3` and upward. A
descriptor the process inherited from its parent works too.

`builtin::out` writes exactly the bytes you give it — no newline is
appended. Add `\n` yourself when you want one.

Descriptors opened by a script are closed when it ends, whether it finishes
or fails.

---

## `.busc` scripts and the bag

A `.busc` file *is* a sequence of pipes and nothing else:

- The **first pipe's** input list is the script's parameter list.
- The **last pipe's** binding is the script's return value.

Every slot in that first list is a parameter, whether it holds a name or a
literal — the caller supplies a value for each one. A named slot also binds
its value for later pipes to use; a literal slot has no name, so its value
is used by the first pipe and not bound.

```
(4: i64, x: i64) : builtin::add -> {r: i64};
```

Two slots, so two arguments: `(4: i64, 10: i64) : bag::addfour -> {r: i64};`

`.busc` scripts are **interpreted every run**, not compiled — the same
tree-walking evaluator BullScript needs for its own prompt is reused to run
a file or a bag call. No build step, no stored binary.

Every callable — builtin or bag entry — needs a declared prototype; there
is no path to registering an arbitrary pre-built binary as a callable name.
The bag stores only `.busc` files.

### How the bag stores things

`bag::add` does its work up front: it reads the file, parses it, type
checks it, and then copies the source into the bag's own directory
(`~/.bullscript/scripts/`). The bag owns its copy, so moving or deleting
the original cannot silently break an entry.

The trade is that editing the original file afterwards does not change the
entry — run `bag::add` again for that.

Parsed scripts are cached in memory, so a bag call inside a chain does not
re-read and re-parse the file every time. Every `bag::` directive refreshes
that cache, so the bag is always current without restarting the prompt.

Entry names must be identifiers: a letter or underscore, then letters,
digits or underscores.

### Directives (typed bare at the prompt)

| Directive | Effect |
|---|---|
| `help` | Print the in-prompt help. |
| `bag::add <path> <name>` | Parse, check and store a `.busc` file under `<name>`. |
| `bag::remove <name>` | Remove a single bag entry. |
| `bag::list` | List your bag entries — builtins never appear here. |
| `record::start` | Start capturing the pipes you type. |
| `record::end` | Stop, preview, and optionally save the recording as a new bag entry. |
| `exit` | Quit. Ctrl+D also works; either discards an in-progress recording with a warning. |

Only lines that parsed, checked and ran successfully are captured by a
recording — a failed line never ends up in a saved script.

Bindings persist across prompt lines, and history is kept between sessions
in `~/.bullscript/history`.

### Builtins

Builtins live in a small, fixed, hardcoded table — never stored in
`bag.json`, never removable via `bag::remove`.

| Builtin | Signature | Notes |
|---|---|---|
| `builtin::add` | `(i64, i64) -> i64` | overflow is an error |
| `builtin::to_upper` / `to_lower` / `trim` | `(String) -> String` | |
| `builtin::out` | `(i64, String) -> bool` | writes verbatim; bool reports success |
| `builtin::in` | `(i64) -> String` | one line, without its newline; empty at end of input |
| `builtin::open` | `(String, String) -> i64` | modes `r` `w` `a` `rw`; failure is an error |
| `builtin::close` | `(i64) -> bool` | false if it wasn't open; 0/1/2 rejected |
| `builtin::run` | `(String) -> bool` | runs a shell command, returns success/failure, discards output |
| `builtin::capture` | `(String) -> String` | runs a shell command, returns stdout, no status info |

`run` and `capture` are separate builtins rather than one: with no tuple
type, a single call can only bind one typed value, so status and output
can't come back from the same call.

---

## Example

```
("notes.txt": String, "w": String)    : builtin::open  -> {fd: i64};
(fd: i64, "first line\n": String)     : builtin::out   -> {ok: bool};
(fd: i64)                             : builtin::close -> {done: bool};
```

```bash
bullscript write_notes.busc notes.txt w "first line
" 
```
