//! Completion: what could come next at the cursor.
//!
//! Shared by the prompt and the language server, so it knows nothing about
//! either. Given the text before the cursor, it works out what kind of thing
//! is being typed and lists the candidates — namespaces, builtin names, bag
//! entries, programs, data fields, types, bindings, or at the prompt, the
//! directives themselves.
//!
//! Context is read off the tail of the line rather than from the parser. A
//! line being completed is by definition unfinished, and the parser rejects
//! unfinished lines; a handful of questions about what precedes the cursor
//! (is there an open `(`? does the word follow `::`? a `:`?) cover the whole
//! grammar, which is small enough for that to be exact rather than a guess.
//!
//! Candidates carry a `detail` — a signature, a type — so a caller that can
//! show it (the hint at the prompt, the popup in an editor) does, and one
//! that cannot simply drops it.

use crate::bag;
use crate::bin_store;
use crate::data;
use crate::lang::builtins;
use crate::lang::types::BsType;

/// One thing the cursor could be completed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The full word: what replaces the partial word being typed.
    pub label:  String,
    /// A signature or a type, for display only.
    pub detail: Option<String>,
}

impl Candidate {
    fn new(label: impl Into<String>) -> Self {
        Candidate { label: label.into(), detail: None }
    }
    fn with(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Candidate { label: label.into(), detail: Some(detail.into()) }
    }
}

/// What `complete` found at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    /// A list of words. `start` is the byte offset in the input at which the
    /// partial word begins; everything from there to the cursor is what the
    /// candidates already match and would replace.
    Words { start: usize, candidates: Vec<Candidate> },
    /// The cursor is in a file-path argument. The engine does not list
    /// directories; the caller has a path completer and uses it.
    Path,
    /// Nothing sensible to offer here.
    None,
}

// ── Prompt directives ─────────────────────────────────────────────────────

/// Every directive accepted at the prompt. Kept here rather than in the
/// prompt module so that a directive and its completion cannot drift apart
/// without someone noticing both lists.
pub const DIRECTIVES: &[&str] = &[
    "help", "clear", "exit",
    "record::start", "record::end",
    "bag::add", "bag::list", "bag::remove", "bag::export", "bag::import",
    "bin::add", "bin::list", "bin::remove",
    "data::add", "data::list", "data::remove", "data::export", "data::import",
];

/// Directives whose first argument is a path on disk.
const PATH_DIRECTIVES: &[&str] = &[
    "bag::add", "bag::export", "bag::import",
    "bin::add",
    "data::add", "data::export", "data::import",
];

// ── Entry point ───────────────────────────────────────────────────────────

/// Complete the word at the end of `before`, the text up to the cursor.
///
/// `bindings` are the names in scope and their types, as `(name, type)`. At
/// the prompt that is the session's environment; in a file it is whatever
/// earlier pipes bound. `at_prompt` enables the directives, which only the
/// prompt accepts.
pub fn complete(before: &str, bindings: &[(String, String)], at_prompt: bool) -> Completion {
    let (start, word) = current_word(before);
    let head = &before[..start];

    // Only whitespace before the word: the line is just beginning.
    if head.trim().is_empty() {
        return if at_prompt {
            words(start, word, DIRECTIVES.iter().map(|d| Candidate::new(*d)).collect())
        } else {
            Completion::None
        };
    }

    // The first argument of a directive: the directive, whitespace, then
    // the word being typed and nothing else. Checked on the text rather than
    // by counting words, because a path like `./tar` is not one word to the
    // tokenizer that found `tar`.
    if at_prompt {
        let first = head.split_whitespace().next().unwrap_or("");
        let rest = &head[first.len()..];
        let in_first_arg = rest.starts_with(char::is_whitespace)
            && !rest.trim_start().contains(char::is_whitespace);
        if in_first_arg {
            if PATH_DIRECTIVES.contains(&first) {
                return Completion::Path;
            }
            let names: Option<Vec<String>> = match first {
                "bag::remove"  => bag::list().ok().map(|l| l.into_iter().map(|(n, _)| n).collect()),
                "bin::remove"  => bin_store::list().ok().map(|l| l.into_iter().map(|(n, _)| n).collect()),
                "data::remove" => data::list().ok().map(|l| l.into_iter().map(|(n, _)| n).collect()),
                _ => None,
            };
            if let Some(names) = names {
                return words(start, word, names.into_iter().map(Candidate::new).collect());
            }
        }
    }

    // Everything else only makes sense inside a pipe.
    if !head.contains('(') {
        return Completion::None;
    }

    let trimmed = head.trim_end();

    // `ns::word` — the word after a namespace. The `::` is part of the word
    // (see current_word), so split it there and complete only the tail.
    if let Some((ns, partial)) = word.rsplit_once("::") {
        let tail_start = start + ns.len() + 2;
        return match namespace_names(ns) {
            Some(cands) => words(tail_start, partial, cands),
            None        => Completion::None,
        };
    }

    // `data::entry.` — a field of a document.
    if let Some(entry) = head.strip_suffix('.')
        .and_then(|h| h.rsplit_once("data::"))
        .map(|(_, e)| e)
        .filter(|e| is_ident(e))
    {
        return words(start, word, document_fields(entry));
    }

    // `x:` or `-> {x:` — a type. Not the `:` after `)`, which introduces
    // the pipe value. The colon may be the last thing typed, in which case
    // it is the tail of the word rather than of the head.
    if word.len() > 1 && word.ends_with(':') && !word.ends_with("::") {
        return words(before.len(), "", types());
    }
    if let Some(before_colon) = trimmed.strip_suffix(':') {
        let slot = !before_colon.ends_with(':') && !before_colon.trim_end().ends_with(')');
        if slot {
            return words(start, word, types());
        }
    }

    // A bare identifier inside the pipe.
    let mut cands: Vec<Candidate> = Vec::new();
    // After the `)` that closes the inputs and its `:`, the pipe value is
    // being typed: a namespace or an expression over the bindings.
    let in_value = head.contains(')') && !head.contains("->");
    if in_value {
        for ns in ["builtin", "bag", "bin", "data"] {
            cands.push(Candidate::with(format!("{}::", ns), ns));
        }
    }
    for (name, ty) in bindings {
        cands.push(Candidate::with(name.clone(), ty.clone()));
    }
    words(start, word, cands)
}

// ── Namespaces ────────────────────────────────────────────────────────────

/// The candidates for a namespace, or None if it is not one.
fn namespace_names(ns: &str) -> Option<Vec<Candidate>> {
    Some(match ns {
        "builtin" => builtin_names(),
        "bag"     => bag_names(),
        "bin"     => bin_names(),
        "data"    => data_names(),
        _         => return None,
    })
}

fn builtin_names() -> Vec<Candidate> {
    builtins::NAMES.iter().map(|n| {
        let detail = builtins::prototype(n)
            .map(|(params, ret)| signature_text(&params, Some(&ret)))
            .unwrap_or_default();
        Candidate::with(*n, detail)
    }).collect()
}

fn bag_names() -> Vec<Candidate> {
    bag::list().unwrap_or_default().into_iter().map(|(n, _)| {
        let detail = bag::signature(&n)
            .map(|s| signature_text(&s.params, s.ret.as_ref()))
            .unwrap_or_else(|_| "does not type check".to_string());
        Candidate::with(n, detail)
    }).collect()
}

fn bin_names() -> Vec<Candidate> {
    bin_store::list().unwrap_or_default().into_iter()
        .map(|(n, _)| Candidate::with(n, "(String...) -> i64"))
        .collect()
}

fn data_names() -> Vec<Candidate> {
    data::list().unwrap_or_default().into_iter()
        .map(|(n, _)| Candidate::with(format!("{}.", n), "document"))
        .collect()
}

/// The top-level fields of a stored document, typed where a field holds a
/// scalar and labelled by JSON kind otherwise.
fn document_fields(entry: &str) -> Vec<Candidate> {
    let Ok(doc) = data::document(entry) else { return Vec::new() };
    let Some(obj) = doc.as_object() else { return Vec::new() };
    obj.iter().map(|(k, v)| {
        let detail = match data::value_type(v) {
            Some(t) => t.to_string(),
            None    => data::json_kind(v).to_string(),
        };
        Candidate::with(k.clone(), detail)
    }).collect()
}

fn types() -> Vec<Candidate> {
    [BsType::I64, BsType::F64, BsType::Bool, BsType::String]
        .iter().map(|t| Candidate::new(t.to_string())).collect()
}

fn signature_text(params: &[BsType], ret: Option<&BsType>) -> String {
    let ps: Vec<String> = params.iter().map(|t| t.to_string()).collect();
    match ret {
        Some(r) => format!("({}) -> {}", ps.join(", "), r),
        None    => format!("({}) -> {{}}", ps.join(", ")),
    }
}

// ── Words ─────────────────────────────────────────────────────────────────

/// Filter `candidates` to those starting with `word`, and package them.
fn words(start: usize, word: &str, candidates: Vec<Candidate>) -> Completion {
    let candidates: Vec<Candidate> = candidates.into_iter()
        .filter(|c| c.label.starts_with(word))
        .collect();
    if candidates.is_empty() {
        Completion::None
    } else {
        Completion::Words { start, candidates }
    }
}

/// The partial word ending at the cursor, and where it starts.
///
/// A word is an identifier, possibly namespaced (`bag::fo`), so `:` is part
/// of it — that is what lets a half-typed `bag::` complete as one unit at
/// the prompt. Dots are not: `data::prompt.au` completes `au`.
fn current_word(before: &str) -> (usize, &str) {
    let start = before
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
        .map(|i| i + c_len(before, i))
        .unwrap_or(0);
    (start, &before[start..])
}

fn c_len(s: &str, i: usize) -> usize {
    s[i..].chars().next().map(char::len_utf8).unwrap_or(1)
}

fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(c: Completion) -> Vec<String> {
        match c {
            Completion::Words { candidates, .. } => candidates.into_iter().map(|c| c.label).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn directives_at_line_start() {
        let l = labels(complete("ba", &[], true));
        assert!(l.iter().all(|s| s.starts_with("ba")));
        assert!(l.contains(&"bag::add".to_string()));
        assert_eq!(labels(complete("ba", &[], false)), Vec::<String>::new());
    }

    #[test]
    fn path_after_add() {
        assert_eq!(complete("bag::add ", &[], true), Completion::Path);
        assert_eq!(complete("bin::add ./tar", &[], true), Completion::Path);
        assert_eq!(complete("bag::add ", &[], false), Completion::None);
    }

    #[test]
    fn builtins_after_namespace() {
        let l = labels(complete("(1: i64) : builtin::t", &[], true));
        assert_eq!(l, vec!["to_upper", "to_lower", "trim"]);
    }

    #[test]
    fn types_after_colon() {
        assert_eq!(labels(complete("(x: ", &[], true)), vec!["i64", "f64", "bool", "String"]);
        assert_eq!(labels(complete("(x: S", &[], true)), vec!["String"]);
        assert_eq!(labels(complete("(1: i64) : x -> {r: ", &[], true)), vec!["i64", "f64", "bool", "String"]);
    }

    #[test]
    fn namespaces_and_bindings_in_value() {
        let b = vec![("total".to_string(), "i64".to_string())];
        let l = labels(complete("(1: i64) : ", &b, true));
        assert!(l.contains(&"builtin::".to_string()));
        assert!(l.contains(&"total".to_string()));
        // Inside the inputs only bindings are offered.
        let l = labels(complete("(to", &b, true));
        assert_eq!(l, vec!["total"]);
    }

    #[test]
    fn current_word_boundaries() {
        assert_eq!(current_word("foo bar"), (4, "bar"));
        assert_eq!(current_word("(1: i64) : bag::ad"), (11, "bag::ad"));
        assert_eq!(current_word("data::prompt.au"), (13, "au"));
        assert_eq!(current_word(""), (0, ""));
    }
}
