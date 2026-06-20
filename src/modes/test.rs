//! `test` — interactively build a tester program for any compiled function.
//!
//! Flow:
//!   1. Filename  → extension inferred for language detection
//!   2. Function name (mandatory)
//!   3. Input/output pairs until `conclude`
//!   4. Tester name
//!   5. Generates, compiles, and globally installs a Rust tester binary
//!      (Java: compiles with javac and runs with java directly)

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

// ── Language detection ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum Lang {
    Rust,
    Python,
    C,
    Cpp,
    Go,
    Java,
}

impl Lang {
    fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "rs"         => Some(Lang::Rust),
            "py"         => Some(Lang::Python),
            "c"          => Some(Lang::C),
            "cpp" | "cc" => Some(Lang::Cpp),
            "go"         => Some(Lang::Go),
            "java"       => Some(Lang::Java),
            _            => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Lang::Rust   => "Rust",
            Lang::Python => "Python",
            Lang::C      => "C",
            Lang::Cpp    => "C++",
            Lang::Go     => "Go",
            Lang::Java   => "Java",
        }
    }
}

// ── Test case ─────────────────────────────────────────────────────────────────

struct TestCase {
    input:    String,
    expected: String,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run() {
    // ── Step 1: filename ──────────────────────────────────────────────────────
    let filename = ask("  file -> ");
    if filename.is_empty() { println!("  Aborted."); return; }

    let ext = Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let lang = match Lang::from_ext(ext) {
        Some(l) => l,
        None => {
            eprintln!("  Unknown extension '.{}'. Supported: rs, py, c, cpp, go, java.", ext);
            return;
        }
    };

    println!("  Language detected: {}", lang.name());

    // ── Step 2: function name ─────────────────────────────────────────────────
    let fn_name = ask("  function -> ");
    if fn_name.is_empty() { println!("  Aborted."); return; }

    // ── Step 3: input/output pairs ────────────────────────────────────────────
    println!("  Enter input/output pairs. Type 'conclude' at any prompt to finish.");
    println!("  Inputs are passed as command-line arguments to the compiled program.");

    let mut cases: Vec<TestCase> = Vec::new();

    loop {
        let input = ask(&format!("  test {} input  -> ", cases.len() + 1));
        if input == "conclude" { break; }
        if input.is_empty() { continue; }

        let expected = ask(&format!("  test {} expect -> ", cases.len() + 1));
        if expected == "conclude" { break; }

        cases.push(TestCase { input, expected });
    }

    if cases.is_empty() {
        println!("  No test cases entered. Aborted.");
        return;
    }

    println!("  {} test case(s) recorded.", cases.len());

    // ── Step 4: tester name ───────────────────────────────────────────────────
    let tester_name = ask("  tester name -> ");
    if tester_name.is_empty() { println!("  Aborted."); return; }

    // ── Step 5: generate, compile, install ───────────────────────────────────
    match lang {
        Lang::Java => run_java_tester(&filename, &fn_name, &cases, &tester_name),
        _          => run_rust_tester(&filename, &fn_name, lang, &cases, &tester_name),
    }
}

// ── Rust-based tester (all non-Java backends) ─────────────────────────────────
//
// Generates a Rust subprocess harness, compiles it with `cargo install`,
// and makes it available globally as `<tester_name>`.

fn run_rust_tester(
    filename:    &str,
    fn_name:     &str,
    lang:        Lang,
    cases:       &[TestCase],
    tester_name: &str,
) {
    let src = generate_rust_tester(filename, fn_name, lang, cases, tester_name);

    let tmp_dir = std::env::temp_dir().join(format!("bullscript_{}", tester_name));
    let src_dir = tmp_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let cargo_toml = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[[bin]]\nname = \"{name}\"\npath = \"src/main.rs\"\n",
        name = tester_name
    );

    fs::write(tmp_dir.join("Cargo.toml"), cargo_toml).unwrap();
    fs::write(src_dir.join("main.rs"), &src).unwrap();

    println!("  Compiling tester...");

    let status = Command::new("cargo")
        .args(["install", "--path", tmp_dir.to_str().unwrap(), "--name", tester_name])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("  Tester '{}' installed globally.", tester_name);
            println!("  Run it with: arrow {} <your_program> result.txt", tester_name);
        }
        Ok(s)  => eprintln!("  Compilation failed (exit {}).", s),
        Err(e) => eprintln!("  Failed to run cargo: {}", e),
    }

    let _ = fs::remove_dir_all(&tmp_dir);
}

// ── Java tester ───────────────────────────────────────────────────────────────
//
// Generates a Java runner class, compiles it with `javac`, and runs it
// immediately — no global install step, since Java binaries aren't
// self-contained executables the way Rust ones are.

fn run_java_tester(
    filename:    &str,
    fn_name:     &str,
    cases:       &[TestCase],
    tester_name: &str,
) {
    let src = generate_java_tester(filename, fn_name, cases, tester_name);

    let tmp_dir   = std::env::temp_dir().join(format!("bullscript_{}", tester_name));
    fs::create_dir_all(&tmp_dir).unwrap();

    // Derive the class name from tester_name (PascalCase)
    let class_name = to_pascal_case(tester_name);
    let java_file  = tmp_dir.join(format!("{}.java", class_name));

    fs::write(&java_file, &src).unwrap();

    // Also copy the target .java file into the tmp dir so javac can see it
    let target_class = to_pascal_case(
        Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Target")
    );
    if let Ok(target_src) = fs::read_to_string(filename) {
        fs::write(tmp_dir.join(format!("{}.java", target_class)), target_src).unwrap();
    }

    println!("  Compiling tester with javac...");

    let compile = Command::new("javac")
        .arg(java_file.to_str().unwrap())
        .arg(tmp_dir.join(format!("{}.java", target_class)).to_str().unwrap())
        .current_dir(&tmp_dir)
        .status();

    match compile {
        Ok(s) if s.success() => {
            println!("  Running tester...\n");
            let run = Command::new("java")
                .arg(&class_name)
                .current_dir(&tmp_dir)
                .status();
            match run {
                Ok(_)  => {}
                Err(e) => eprintln!("  Failed to run java: {}", e),
            }
        }
        Ok(s)  => eprintln!("  javac failed (exit {}).", s),
        Err(e) => eprintln!("  Failed to run javac: {}", e),
    }

    let _ = fs::remove_dir_all(&tmp_dir);
}

// ── Tester source generation ──────────────────────────────────────────────────

fn generate_rust_tester(
    filename: &str,
    fn_name:  &str,
    lang:     Lang,
    cases:    &[TestCase],
    name:     &str,
) -> String {
    let mut src = String::new();

    src.push_str("use std::process::Command;\n\n");
    src.push_str(&format!(
        "// Tester for `{}` in `{}` ({}), generated by bullscript test\n\n",
        fn_name, filename, lang.name()
    ));
    src.push_str("fn main() {\n");
    src.push_str("    let program = std::env::args().nth(1)\n");
    src.push_str(&format!(
        "        .expect(\"Usage: {} <program>\");\n\n",
        name
    ));
    src.push_str("    let cases: &[(&str, &str)] = &[\n");

    for case in cases {
        src.push_str(&format!(
            "        ({:?}, {:?}),\n",
            case.input, case.expected
        ));
    }

    src.push_str("    ];\n\n");
    src.push_str("    let mut passed = 0usize;\n");
    src.push_str("    let mut failed = 0usize;\n\n");
    src.push_str("    for (input, expected) in cases {\n");
    src.push_str("        let args: Vec<&str> = input.split_whitespace().collect();\n");
    src.push_str("        let output = Command::new(&program)\n");
    src.push_str("            .args(&args)\n");
    src.push_str("            .output()\n");
    src.push_str("            .expect(\"Failed to run program\");\n\n");
    src.push_str("        let stdout = String::from_utf8_lossy(&output.stdout);\n");
    src.push_str("        let actual = stdout.trim();\n\n");
    src.push_str("        if actual == *expected {\n");
    src.push_str("            println!(\"  ok    {} -> {}\", input, expected);\n");
    src.push_str("            passed += 1;\n");
    src.push_str("        } else {\n");
    src.push_str("            println!(\"  FAIL  {} -> expected: {}, got: {}\", input, expected, actual);\n");
    src.push_str("            failed += 1;\n");
    src.push_str("        }\n");
    src.push_str("    }\n\n");
    src.push_str("    println!();\n");
    src.push_str("    println!(\"  {} passed, {} failed\", passed, failed);\n");
    src.push_str("    if failed > 0 { std::process::exit(1); }\n");
    src.push_str("}\n");

    src
}

fn generate_java_tester(
    filename: &str,
    fn_name:  &str,
    cases:    &[TestCase],
    name:     &str,
) -> String {
    let class_name   = to_pascal_case(name);
    let target_class = to_pascal_case(
        Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Target")
    );

    let mut src = String::new();

    src.push_str(&format!(
        "// Tester for `{}` in `{}` (Java), generated by bullscript test\n\n",
        fn_name, filename
    ));
    src.push_str(&format!("public class {} {{\n\n", class_name));
    src.push_str("    public static void main(String[] args) {\n");
    src.push_str("        int passed = 0;\n");
    src.push_str("        int failed = 0;\n\n");

    // Each test case calls the static method directly.
    // Inputs are parsed as long (most common Bullang numeric type).
    // If the function takes a String, the user would have entered a string arg.
    for (i, case) in cases.iter().enumerate() {
        let inputs: Vec<&str> = case.input.split_whitespace().collect();
        let args_java = inputs.iter().map(|a| {
            // Heuristic: if it looks like a number pass as-is, else as a quoted String
            if a.parse::<f64>().is_ok() {
                a.to_string()
            } else {
                format!("\"{}\"", a)
            }
        }).collect::<Vec<_>>().join(", ");

        src.push_str(&format!(
            "        // test {i}\n"
        ));
        src.push_str(&format!(
            "        var __actual{i} = String.valueOf({target}.{fn_name}({args}));\n",
            i       = i,
            target  = target_class,
            fn_name = fn_name,
            args    = args_java,
        ));
        src.push_str(&format!(
            "        var __expected{i} = \"{expected}\";\n",
            i        = i,
            expected = case.expected.replace('"', "\\\""),
        ));
        src.push_str(&format!(
            "        if (__actual{i}.equals(__expected{i})) {{\n"
        ));
        src.push_str(&format!(
            "            System.out.println(\"  ok    {input} -> {expected}\");\n",
            input    = case.input.replace('"', "\\\""),
            expected = case.expected.replace('"', "\\\""),
        ));
        src.push_str("            passed++;\n");
        src.push_str("        } else {\n");
        src.push_str(&format!(
            "            System.out.printf(\"  FAIL  {input} -> expected: {expected}, got: %s%n\", __actual{i});\n",
            input    = case.input.replace('"', "\\\""),
            expected = case.expected.replace('"', "\\\""),
            i        = i,
        ));
        src.push_str("            failed++;\n");
        src.push_str("        }\n\n");
    }

    src.push_str("        System.out.println();\n");
    src.push_str("        System.out.printf(\"  %d passed, %d failed%n\", passed, failed);\n");
    src.push_str("        if (failed > 0) System.exit(1);\n");
    src.push_str("    }\n}\n");

    src
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ask(label: &str) -> String {
    print!("{}", label);
    io::stdout().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap_or(0);
    buf.trim().to_string()
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut upper  = true;
    for ch in s.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            result.extend(ch.to_uppercase());
            upper = false;
        } else {
            result.push(ch);
        }
    }
    result
}
