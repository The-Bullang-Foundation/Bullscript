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

Runs a `.busc` file non-interactively. The script's first pipe declares
its parameters: each **named** slot takes one argument from the command
line, parsed into the slot's declared type, and a mismatch is reported
before anything runs. A literal slot is a value the script already holds,
so it takes nothing. When the script finishes, its return value — the
last pipe's binding — is printed on its own line, so `$(bullscript
x.busc)` is that value; a script that ends in `-> {}` prints nothing of
its own.

```bash
bullscript lsp
```

Runs the language server on stdin/stdout, for the editor extensions in
`bullscript-vscode/` and `zed-bullscript/`. It reports the same errors the
prompt would, and offers the same completions.

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
- An input is a literal, a binding already in scope, or a field of a
  stored document (`data::entry.field`, see below).
- The middle section is either a call (`builtin::name`, `bag::name` or
  `bin::name`, taking the pipe's inputs as its arguments, in order) or a
  bare arithmetic/comparison/logical expression over the pipe's own
  inputs (`+ - * /`, `== != < > <= >=`, `&& ||`, unary `-`/`!`, parens).
  An expression may leave inputs unused.
- `-> {}` discards the result. `-> {name: type}` creates or overwrites
  `name` with the computed value. `-> {data::entry.field: type}` writes it
  into a stored document instead.
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

When a script is called from a pipe as a bag entry, every slot in that
first list is a parameter, whether it holds a name or a literal — the
caller fills each one. A named slot also binds its value for later pipes to
use; a literal slot has no name, so its value is used by the first pipe and
not bound.

```
(4: i64, x: i64) : builtin::add -> {r: i64};
```

Two slots, so two arguments: `(4: i64, 10: i64) : bag::addfour -> {r: i64};`

From the command line there is no caller, so only the named slots are
parameters; a literal slot keeps the value the script wrote. The same
script run directly takes one argument: `bullscript addfour.busc 10`.

`.busc` scripts are **interpreted every run**, not compiled — the same
tree-walking evaluator BullScript needs for its own prompt is reused to run
a file or a bag call. No build step, no stored binary.

Every callable — builtin or bag entry — needs a declared prototype. The
bag stores only `.busc` files; a pre-built program goes in the bin store,
which gives it a fixed prototype of its own (below).

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

---

## Builtins

Builtins live in a small, fixed, hardcoded table — never stored in
`bag.json`, never removable via `bag::remove`.

| Builtin | Signature | Notes |
|---|---|---|
| `builtin::add` | `(i64, i64) -> i64` | overflow is an error |
| `builtin::to_upper` / `to_lower` / `trim` | `(String) -> String` | |
| `builtin::i64_to_str` | `(i64) -> String` | |
| `builtin::str_to_i64` | `(String) -> i64` | `0` when the text is not a number, not an error |
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

## The data store

`data::add` stores a `.json` file the same way the bag stores a script:
parsed up front (it must be a JSON object), copied into
`~/.bullscript/data/`, cached in memory and refreshed by every `data::`
directive.

A pipe reads a field of a document as an input, and writes one as a
binding:

```
(1: i64, data::prompt.audit: String) : builtin::out -> {};
("new text": String) : builtin::trim -> {data::prompt.audit: String};
```

A field keeps the type it has in the document — a JSON string is a
`String`, an integral number an `i64`, a number with a fraction an `f64`, a
boolean a `bool` — and the declared type must match. Nested fields chain
with dots, and `data::entry[name]` selects a field by the `String` a
binding holds at run time. A field must already exist: reading or writing a
missing one is an error, and a write takes effect when its pipe runs, like
`builtin::out`.

---

## The bin store

`bin::add <path> <name>` copies an already-built program into
`~/.bullscript/bin/` — a compiled binary, or a script with a shebang line.
Building it is yours to do; BullScript keeps only the result. It is then
callable from any pipe:

```
("--check": String) : bin::mytool -> {code: i64};
```

Every program has the same prototype: any number of `String` arguments,
passed as separate argv entries and never through a shell, and its exit
code back as an `i64`. The program inherits the terminal, so it prints
where you can see it and can read input.

There is deliberately no `bin::export` or `bin::import`: a compiled program
is tied to one operating system and one architecture, so an archive of
binaries handed to someone else would largely not run.

---

## The prompt

### Directives (typed bare at the prompt)

| Directive | Effect |
|---|---|
| `help` | Print the in-prompt help. |
| `clear` | Clear the screen. Bindings, stores and history are untouched. |
| `exit` | Quit. Ctrl+D also works; either discards an in-progress recording with a warning. |
| `record::start` | Start capturing the pipes you type. |
| `record::end` | Stop, preview, and optionally save the recording as a new bag entry. |
| `bag::add <path> <name>` | Parse, check and store a `.busc` file under `<name>`. |
| `bag::remove <name>` | Remove a single bag entry. |
| `bag::list` | List your bag entries — builtins never appear here. |
| `bag::export <path>` | Write every script in the bag into one `.zip`. A directory gets `bullscript-bag.zip` inside it. |
| `bag::import <path>` | Read every `.busc` in a `.zip` into the bag, named after each file. Existing names are replaced; a file that does not parse is skipped. |
| `data::add <path> <name>` | Parse and store a `.json` object under `<name>`. |
| `data::remove <name>` | Remove a single document. |
| `data::list` | List your documents. |
| `data::export <path>` / `data::import <path>` | As for the bag, with `.json` files and `bullscript-data.zip`. |
| `bin::add <path> <name>` | Copy a built program into the bin store under `<name>`. |
| `bin::remove <name>` | Remove a single program. |
| `bin::list` | List your programs. |

Only lines that parsed, checked and ran successfully are captured by a
recording — a failed line never ends up in a saved script.

Bindings persist across prompt lines, and history is kept between sessions
in `~/.bullscript/history`.

The prompt completes as you type. Tab lists what fits at the cursor —
directives at the start of a line, file paths after `bag::add` and the other
directives that take one, then inside a pipe: `builtin::`, `bag::`, `bin::`
and `data::` names with their signatures, the top-level fields of a `data::`
document after its dot, types after a `:`, and your bindings. When only one
thing fits, or every candidate shares a longer prefix, the rest is shown
dimmed after the cursor; the right arrow accepts it.

---

## Example

`write_notes.busc`:

```
(path: String, line: String)  : path           -> {p: String};
(p: String, "w": String)      : builtin::open  -> {fd: i64};
(fd: i64, line: String)       : builtin::out   -> {ok: bool};
(fd: i64)                     : builtin::close -> {done: bool};
```

The first pipe declares the script's two parameters. `builtin::open` only
takes a path and a mode, and the mode is the script's own business, so the
parameters are declared in an expression pipe — which may carry inputs it
does not use — and `open` gets its literal `"w"` on the next line.

```bash
bullscript write_notes.busc notes.txt "first line
"
```

Stored in the bag, the same script is called with its literal slots filled
too, since the caller is another pipe:

```
(path: String, line: String) : bag::write_notes -> {done: bool};
```
