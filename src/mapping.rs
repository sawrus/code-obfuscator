use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::fs_ops::FileEntry;
use crate::language::{detect_language, is_keyword};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MappingFile {
    pub forward: BTreeMap<String, String>,
    pub reverse: BTreeMap<String, String>,
}

pub fn load_manual(path: Option<&Path>) -> AppResult<BTreeMap<String, String>> {
    let Some(path) = path else {
        return Ok(BTreeMap::new());
    };
    let raw = fs::read_to_string(path)?;
    let parsed: BTreeMap<String, String> = serde_json::from_str(&raw)?;
    Ok(parsed)
}

pub fn load_mapping(path: &Path) -> AppResult<MappingFile> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_mapping(path: &Path, forward: &BTreeMap<String, String>) -> AppResult<()> {
    let data = MappingFile {
        forward: forward.clone(),
        reverse: invert(forward)?,
    };
    let text = serde_json::to_string_pretty(&data)?;
    fs::write(path, text)?;
    Ok(())
}

pub fn invert(map: &BTreeMap<String, String>) -> AppResult<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (k, v) in map {
        if out.insert(v.clone(), k.clone()).is_some() {
            return err_dup(v);
        }
    }
    Ok(out)
}

fn err_dup(v: &str) -> AppResult<BTreeMap<String, String>> {
    Err(AppError::InvalidArg(format!("duplicate mapped value: {v}")))
}

pub fn detect_terms(files: &[FileEntry]) -> AppResult<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*")?;
    for file in files {
        let lang = detect_language(&file.rel, &file.text);
        collect_terms(&re, &file.text, lang, &mut out);
    }
    Ok(out)
}

fn collect_terms(
    re: &Regex,
    text: &str,
    lang: crate::language::Language,
    out: &mut BTreeSet<String>,
) {
    for token in code_identifier_tokens(text, lang) {
        if let Some(m) = re.find(token) {
            if m.start() != 0 || m.end() != token.len() {
                continue;
            }
        } else {
            continue;
        }

        if is_candidate_identifier(token, lang) {
            out.insert(token.to_string());
        }
    }
}

fn is_candidate_identifier(s: &str, lang: crate::language::Language) -> bool {
    if s.len() < 3 {
        return false;
    }
    if is_keyword(lang, s) || is_system_name(lang, s) {
        return false;
    }
    is_valid_identifier_for_language(lang, s)
}

fn is_valid_identifier_for_language(lang: crate::language::Language, s: &str) -> bool {
    match lang {
        crate::language::Language::Bash => {
            let mut chars = s.chars();
            matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
                && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        }
        crate::language::Language::Unknown => {
            let mut chars = s.chars();
            matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
                && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        }
        _ => {
            let mut chars = s.chars();
            matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
                && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        }
    }
}

fn is_system_name(lang: crate::language::Language, s: &str) -> bool {
    match lang {
        crate::language::Language::Python => {
            (s.starts_with("__") && s.ends_with("__")) || PYTHON_RUNTIME_DENYLIST.contains(&s)
        }
        crate::language::Language::Sql => {
            SQL_SYSTEM_DENYLIST.contains(&s)
                || SQL_SYSTEM_DENYLIST
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(s))
        }
        crate::language::Language::Bash => {
            BASH_SYSTEM_DENYLIST.contains(&s)
                || s.chars().all(|c| c.is_ascii_digit())
                || (s.starts_with('_') && s[1..].chars().all(|c| c.is_ascii_digit()))
        }
        _ => false,
    }
}

const PYTHON_RUNTIME_DENYLIST: &[&str] = &[
    "self",
    "cls",
    "__name__",
    "__main__",
    "__file__",
    "__package__",
    "__dict__",
    "__all__",
    "__doc__",
];

const SQL_SYSTEM_DENYLIST: &[&str] = &[
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "now",
    "current_date",
    "current_time",
    "coalesce",
    "upper",
    "lower",
    "substring",
    "cast",
    "json_extract",
    "row_number",
];

const BASH_SYSTEM_DENYLIST: &[&str] = &[
    "IFS",
    "PATH",
    "HOME",
    "PWD",
    "OLDPWD",
    "UID",
    "EUID",
    "PPID",
    "SHELL",
    "BASH",
    "BASH_VERSION",
    "RANDOM",
    "LINENO",
    "SECONDS",
    "OPTIND",
    "OPTARG",
    "FUNCNAME",
    "PIPESTATUS",
];

fn code_identifier_tokens<'a>(text: &'a str, lang: crate::language::Language) -> Vec<&'a str> {
    match lang {
        crate::language::Language::Python => lex_python_tokens(text),
        crate::language::Language::Sql => lex_sql_tokens(text),
        crate::language::Language::Bash => lex_bash_tokens(text),
        _ => lex_c_like_tokens(text),
    }
}

fn lex_c_like_tokens(text: &str) -> Vec<&str> {
    enum State {
        Code,
        LineComment,
        BlockComment,
        String(char),
    }
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut state = State::Code;
    while i < bytes.len() {
        match state {
            State::Code => {
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = State::LineComment;
                    i += 2;
                    continue;
                }
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = State::BlockComment;
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    state = State::String(bytes[i] as char);
                    i += 1;
                    continue;
                }
                if is_ident_start(bytes[i]) {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && is_ident_char(bytes[i]) {
                        i += 1;
                    }
                    out.push(&text[start..i]);
                    continue;
                }
                i += 1;
            }
            State::LineComment => {
                if bytes[i] == b'\n' {
                    state = State::Code;
                }
                i += 1;
            }
            State::BlockComment => {
                if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = State::Code;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            State::String(q) => {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] as char == q {
                    state = State::Code;
                }
                i += 1;
            }
        }
    }
    out
}

fn lex_python_tokens(text: &str) -> Vec<&str> {
    enum State {
        Code,
        Comment,
        String(char),
    }
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut state = State::Code;
    while i < bytes.len() {
        match state {
            State::Code => {
                if bytes[i] == b'#' {
                    state = State::Comment;
                    i += 1;
                    continue;
                }
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    state = State::String(bytes[i] as char);
                    i += 1;
                    continue;
                }
                if is_ident_start(bytes[i]) {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && is_ident_char(bytes[i]) {
                        i += 1;
                    }
                    out.push(&text[start..i]);
                    continue;
                }
                i += 1;
            }
            State::Comment => {
                if bytes[i] == b'\n' {
                    state = State::Code;
                }
                i += 1;
            }
            State::String(q) => {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] as char == q {
                    state = State::Code;
                }
                i += 1;
            }
        }
    }
    out
}

fn lex_sql_tokens(text: &str) -> Vec<&str> {
    enum State {
        Code,
        LineComment,
        BlockComment,
        String,
    }
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut state = State::Code;
    while i < bytes.len() {
        match state {
            State::Code => {
                if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                    state = State::LineComment;
                    i += 2;
                    continue;
                }
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = State::BlockComment;
                    i += 2;
                    continue;
                }
                if bytes[i] == b'\'' {
                    state = State::String;
                    i += 1;
                    continue;
                }
                if is_ident_start(bytes[i]) {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && is_ident_char(bytes[i]) {
                        i += 1;
                    }
                    out.push(&text[start..i]);
                    continue;
                }
                i += 1;
            }
            State::LineComment => {
                if bytes[i] == b'\n' {
                    state = State::Code;
                }
                i += 1;
            }
            State::BlockComment => {
                if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = State::Code;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            State::String => {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        i += 2;
                        continue;
                    }
                    state = State::Code;
                }
                i += 1;
            }
        }
    }
    out
}

fn lex_bash_tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_comment = false;

    while i < bytes.len() {
        if in_comment {
            if bytes[i] == b'\n' {
                in_comment = false;
            }
            i += 1;
            continue;
        }
        if in_single {
            if bytes[i] == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }

        if bytes[i] == b'#' {
            in_comment = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'\'' {
            in_single = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            in_double = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'$' {
            if i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                i += 2;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                continue;
            }
            if i + 1 < bytes.len() && is_ident_start(bytes[i + 1]) {
                i += 1;
                let start = i;
                i += 1;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                out.push(&text[start..i]);
                continue;
            }
        }
        if is_ident_start(bytes[i]) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            out.push(&text[start..i]);
            continue;
        }
        i += 1;
    }

    out
}

fn is_ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

fn is_ident_char(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

pub fn enrich_with_random(
    map: &mut BTreeMap<String, String>,
    terms: &BTreeSet<String>,
    seed: Option<u64>,
) {
    let mut rng = seeded(seed);
    let mut used = used_values(map);
    for term in terms {
        maybe_insert(term, map, &mut used, &mut rng);
    }
}

fn seeded(seed: Option<u64>) -> StdRng {
    let val = seed.unwrap_or_else(rand::random);
    StdRng::seed_from_u64(val)
}

fn used_values(map: &BTreeMap<String, String>) -> BTreeSet<String> {
    map.values().cloned().collect()
}

fn maybe_insert(
    term: &str,
    map: &mut BTreeMap<String, String>,
    used: &mut BTreeSet<String>,
    rng: &mut StdRng,
) {
    if map.contains_key(term) {
        return;
    }
    let value = next_unique(used, rng);
    map.insert(term.to_string(), value);
}

fn next_unique(used: &mut BTreeSet<String>, rng: &mut StdRng) -> String {
    loop {
        let candidate = format!("{}{}", pick(rng), rng.random_range(1000..9999));
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
}

fn pick(rng: &mut StdRng) -> &'static str {
    let idx = rng.random_range(0..PREFIX.len());
    PREFIX[idx]
}

const PREFIX: &[&str] = &[
    "Amber", "Cedar", "Quartz", "Falcon", "Maple", "Nimbus", "Atlas", "Comet", "Coral", "River",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_reverse_map() {
        let mut f = BTreeMap::new();
        f.insert("Freeze".to_string(), "Go".to_string());
        let rev = invert(&f).expect("reverse map");
        assert_eq!(rev.get("Go"), Some(&"Freeze".to_string()));
    }

    #[test]
    fn detects_identifiers_and_skips_keywords_for_python() {
        let terms = detect_terms(&[FileEntry {
            rel: "main.py".into(),
            text: "def Freeze(antifraud_check):\n    return antifraud_check".into(),
        }])
        .expect("terms");
        assert!(terms.contains("Freeze"));
        assert!(terms.contains("antifraud_check"));
        assert!(!terms.contains("def"));
        assert!(!terms.contains("return"));
    }

    #[test]
    fn detects_identifiers_for_sql() {
        let terms = detect_terms(&[FileEntry {
            rel: "schema.sql".into(),
            text: "SELECT user_id FROM accounts WHERE status = 'active';".into(),
        }])
        .expect("terms");
        assert!(terms.contains("user_id"));
        assert!(terms.contains("accounts"));
        assert!(!terms.contains("SELECT"));
        assert!(!terms.contains("FROM"));
    }

    #[test]
    fn supports_top_10_languages_keyword_filtering() {
        let fixtures = vec![
            (
                "a.py",
                "def CustomerName(value):
  return value",
                "def",
                "CustomerName",
            ),
            (
                "a.js",
                "function processOrder(input) { return input; }",
                "function",
                "processOrder",
            ),
            (
                "a.ts",
                "interface UserModel { id: string } const trackEvent = 1",
                "interface",
                "trackEvent",
            ),
            (
                "a.java",
                "public class PaymentService { int score; }",
                "class",
                "PaymentService",
            ),
            (
                "a.cs",
                "public class FraudEngine { private int score; }",
                "class",
                "FraudEngine",
            ),
            (
                "a.cpp",
                "class Detector { int score; };",
                "class",
                "Detector",
            ),
            (
                "a.go",
                "func BuildReport() { var customerId int }",
                "func",
                "BuildReport",
            ),
            (
                "a.rs",
                "fn build_report() { let customer_id = 1; }",
                "fn",
                "build_report",
            ),
            (
                "a.sql",
                "SELECT account_id FROM ledger",
                "SELECT",
                "account_id",
            ),
            (
                "a.sh",
                "function deploy_app() { local env=prod; }",
                "function",
                "deploy_app",
            ),
        ];

        for (path, text, kw, ident) in fixtures {
            let terms = detect_terms(&[FileEntry {
                rel: path.into(),
                text: text.into(),
            }])
            .expect("terms");
            assert!(terms.contains(ident), "missing {ident} for {path}");
            assert!(
                !terms.contains(kw),
                "keyword {kw} should be filtered for {path}"
            );
        }
    }

    #[test]
    fn safely_skips_strings_and_comments() {
        let terms = detect_terms(&[FileEntry {
            rel: "main.py".into(),
            text: "# CustomerName comment\ntext = \"CustomerName in string\"\nactual_name = 1\n"
                .into(),
        }])
        .expect("terms");
        assert!(!terms.contains("comment"));
        assert!(!terms.contains("string"));
        assert!(!terms.contains("CustomerName"));
        assert!(terms.contains("actual_name"));
    }

    #[test]
    fn skips_language_specific_system_names() {
        let py_terms = detect_terms(&[FileEntry {
            rel: "main.py".into(),
            text: "def __init__(self):\n    return __name__\n".into(),
        }])
        .expect("py terms");
        assert!(!py_terms.contains("__init__"));
        assert!(!py_terms.contains("self"));
        assert!(!py_terms.contains("__name__"));

        let sql_terms = detect_terms(&[FileEntry {
            rel: "schema.sql".into(),
            text: "SELECT COUNT(user_id), ledger_name FROM accounts;".into(),
        }])
        .expect("sql terms");
        assert!(!sql_terms.contains("COUNT"));
        assert!(sql_terms.contains("ledger_name"));

        let sh_terms = detect_terms(&[FileEntry {
            rel: "script.sh".into(),
            text: "echo $1 $PATH\nmy_var=1\n".into(),
        }])
        .expect("bash terms");
        assert!(!sh_terms.contains("PATH"));
        assert!(!sh_terms.contains("1"));
        assert!(sh_terms.contains("my_var"));
    }
}
