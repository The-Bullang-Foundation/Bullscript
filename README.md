# Bullscript

Bullscript is the interactive writing and testing tool for [Bullang](https://github.com/The-Bullang-Foundation/Bullang). It guides you through building `.bu` functions step by step and generating testers for compiled code.

It depends on Bullang as a library crate and can be installed independently.

---

## Prerequisite

Cargo v1.92.0 or later.

## Installation

```bash
cargo install --git https://github.com/The-Bullang-Foundation/Bullscript.git
```

If you are reinstalling over an existing version, add `--force`:

```bash
cargo install --git https://github.com/The-Bullang-Foundation/Bullscript.git --force bullscript
```

After installation, `bullscript` is available from anywhere.

---

## Usage

```bash
bullscript
```

Launches the interactive prompt:

```
command ->
```
### `update`

Reinstall Bullscript from the latest commit on the main branch.

```bash
bullscript update
```

---

## Commands

### `run`

Execute a Bullang source file directly. No transpilation or compilation needed.

The file must define a zero-argument `main` bullet as its entry point. Use `builtin::out` for any output — there is no implicit printing.

```
command -> run script.bu
command -> run path/to/my_program.bu
```

Bullscript checks for native escape blocks before running. If any are found, it reports each offending bullet and suggests `bullarchy convert` as the alternative.

```
Error: the following bullets contain native escape blocks and cannot be interpreted:
  'add_vec' — @rust
  'sort'    — @c

Remove the escape blocks or use 'bullarchy convert' to transpile instead.
```

---

### `build`

Enter build mode. Bullscript guides you through writing a `.bu` function step by step:

1. **File** — name of the `.bu` file to create or append to.
2. **Prototype** — the function signature, e.g. `let add(a: i32, b: i32) -> result: i32`.
3. **Pipes** — up to 5 pipe statements. Type `conclude` to finish early.

The assembled function is parsed, pretty-printed, and shown for review before you choose whether to save it.

```
command -> build
  file -> my_file.bu
  prototype -> let add(a: i32, b: i32) -> result: i32
  pipe 1 -> (a, b) : a + b -> {result};
  pipe 2 -> conclude

  Preview:

  let add(a: i32, b: i32) -> result: i32 {
      (a, b) : a + b -> {result};
  }

  Save? (Y/n) ->
```

---

### `test`

Interactively build a tester program for a compiled function. Bullscript prompts you for:

1. **Filename** — language is inferred from the extension (`rs`, `py`, `c`, `cpp`, `go`).
2. **Function name** — the specific function to test.
3. **Input/output pairs** — type `conclude` to finish.
4. **Tester name** — Bullscript compiles and globally installs a Rust tester binary.

The generated tester takes your compiled program as an argument, runs it with each set of inputs, compares the output to the expected value, and reports pass/fail per case.

```
command -> test
  file -> add.rs
  Language detected: Rust
  function -> add
  Enter input/output pairs. Type 'conclude' at any prompt to finish.
  test 1 input  -> 1 2
  test 1 expect -> 3
  test 2 input  -> 0 0
  test 2 expect -> 0
  test 3 input  -> conclude
  2 test case(s) recorded.
  tester name -> add_tester
  Compiling tester...
  Tester 'add_tester' installed globally.
  Run it with: arrow add_tester <your_program> result.txt
```

Then run it with `arrow`:

```
command -> arrow add_tester ./add result.txt
```

---

### `arrow`

Apply one thing on another and write the result to a file. Two behaviors, resolved automatically from the second argument:

- **Second arg is a program** → pipe the first program's stdout into the second program's stdin.
- **Second arg is a file** → feed the file as stdin to the first program.

Always writes a result file.

```
command -> arrow <first> <second> <output_file>
```

Examples:

```
command -> arrow ./gen_input ./my_program result.txt
command -> arrow ./my_program input.txt result.txt
command -> arrow add_tester ./add result.txt
```

---

### `help`

Print the list of available commands.

```
command -> help
```

---

### `exit`

Quit Bullscript.

```
command -> exit
```

---

## Relationship to Bullang and Bullarchy

**[Bullang](https://github.com/The-Bullang-Foundation/Bullang)** is the language definition — grammar, AST, parser, formatter, and stdlib catalogue. Bullscript depends on it as a library crate. Run `bullang stdlib` to browse available builtins.

**[Bullarchy](https://github.com/The-Bullang-Foundation/Bullarchy)** is the transpiler and project manager. Bullscript does not convert `.bu` to a target language — that is Bullarchy's role. The two tools complement each other: write and test with Bullscript, transpile and integrate with Bullarchy.
