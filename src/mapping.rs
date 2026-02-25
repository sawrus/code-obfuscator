use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::fs_ops::FileEntry;
use crate::language::{Language, detect_language, is_keyword, is_valid_identifier_for};

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
    let re = Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]{2,}\b")?;
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
    let python_params = if matches!(lang, Language::Python) {
        collect_python_parameter_names(text)
    } else {
        BTreeSet::new()
    };

    for m in re.find_iter(text) {
        let s = m.as_str();
        if !is_keyword(lang, s) && !is_reserved_identifier(lang, s) && !python_params.contains(s) {
            out.insert(s.to_string());
        }
    }
}

fn collect_python_parameter_names(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let def_re = Regex::new(r"\bdef\s+[A-Za-z_][A-Za-z0-9_]*\s*\(([^)]*)\)").expect("regex");
    let ident_re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("regex");

    for cap in def_re.captures_iter(text) {
        let params = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        for ident in ident_re.find_iter(params).map(|m| m.as_str()) {
            if matches!(ident, "self" | "cls") {
                continue;
            }
            out.insert(ident.to_string());
        }
    }

    out
}

fn is_reserved_identifier(_lang: crate::language::Language, s: &str) -> bool {
    (s.starts_with("__") && s.ends_with("__"))
        || matches!(
            s,
            "main"
                | "self"
                | "cls"
                | "print"
                | "printf"
                | "println"
                | "Println"
                | "Main"
                | "Program"
                | "String"
                | "args"
                | "str"
                | "string"
                | "std"
                | "echo"
        )
}

pub fn enrich_with_random(
    map: &mut BTreeMap<String, String>,
    terms: &BTreeSet<String>,
    files: &[FileEntry],
    _seed: Option<u64>,
) {
    let mut used = used_values(map);
    let (term_namespaces, namespace_terms) = collect_namespace_terms(files);
    let kinds = collect_identifier_kinds(files);
    let mut sequence = NameSequence::default();
    for term in terms {
        maybe_insert(
            term,
            map,
            &mut used,
            &mut sequence,
            &term_namespaces,
            &namespace_terms,
            kinds.get(term),
        );
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum IdentifierKind {
    PyClass,
    PyMethod,
    PyField,
    PyVar,
    PyConst,
    SqlSchema,
    SqlTable,
    GoVar,
    Generic,
}

fn collect_namespace_terms(
    files: &[FileEntry],
) -> (
    BTreeMap<String, BTreeSet<String>>,
    BTreeMap<String, BTreeSet<String>>,
) {
    let re = Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]{2,}\b").expect("valid regex");
    let mut term_namespaces: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut namespace_terms: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for file in files {
        let lang = detect_language(&file.rel, &file.text);
        let ns = namespace_for(file);
        for m in re.find_iter(&file.text) {
            let s = m.as_str();
            if is_keyword(lang, s) {
                continue;
            }
            namespace_terms
                .entry(ns.clone())
                .or_default()
                .insert(s.to_string());
            term_namespaces
                .entry(s.to_string())
                .or_default()
                .insert(ns.clone());
        }
    }
    (term_namespaces, namespace_terms)
}

fn namespace_for(file: &FileEntry) -> String {
    let parent = file.rel.parent().unwrap_or_else(|| Path::new(""));
    let ext = file
        .rel
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    format!("{}::{}", parent.display(), ext)
}

fn used_values(map: &BTreeMap<String, String>) -> BTreeSet<String> {
    map.values().cloned().collect()
}

fn maybe_insert(
    term: &str,
    map: &mut BTreeMap<String, String>,
    used: &mut BTreeSet<String>,
    sequence: &mut NameSequence,
    term_namespaces: &BTreeMap<String, BTreeSet<String>>,
    namespace_terms: &BTreeMap<String, BTreeSet<String>>,
    kind: Option<&IdentifierKind>,
) {
    if map.contains_key(term) {
        return;
    }
    let value = next_unique(term, used, sequence, term_namespaces, namespace_terms, kind);
    map.insert(term.to_string(), value);
}

fn next_unique(
    term: &str,
    used: &mut BTreeSet<String>,
    sequence: &mut NameSequence,
    term_namespaces: &BTreeMap<String, BTreeSet<String>>,
    namespace_terms: &BTreeMap<String, BTreeSet<String>>,
    kind: Option<&IdentifierKind>,
) -> String {
    let namespaces = term_namespaces.get(term);
    let prefix = select_prefix(term, kind.copied().unwrap_or(IdentifierKind::Generic));

    loop {
        let candidate = sequence.next(&prefix);
        if !is_valid_identifier_for(Language::Unknown, &candidate) {
            continue;
        }
        if !is_namespace_safe(&candidate, namespaces, namespace_terms) {
            continue;
        }
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
}

fn is_namespace_safe(
    candidate: &str,
    namespaces: Option<&BTreeSet<String>>,
    namespace_terms: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    let Some(namespaces) = namespaces else {
        return true;
    };
    namespaces.iter().all(|ns| {
        namespace_terms
            .get(ns)
            .map(|existing| !existing.contains(candidate))
            .unwrap_or(true)
    })
}

fn select_prefix(term: &str, kind: IdentifierKind) -> String {
    let raw = match kind {
        IdentifierKind::PyClass => "py_class",
        IdentifierKind::PyMethod => "py_method",
        IdentifierKind::PyField => "py_field",
        IdentifierKind::PyVar => "py_var",
        IdentifierKind::PyConst => "py_const",
        IdentifierKind::SqlSchema => "sql_schema",
        IdentifierKind::SqlTable => "sql_table",
        IdentifierKind::GoVar => "go_var",
        IdentifierKind::Generic => "py_var",
    };

    if term.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
        return raw.to_ascii_uppercase();
    }
    if term.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return raw
            .split('_')
            .map(|part| {
                let mut chars = part.chars();
                let first = chars.next().unwrap_or_default().to_ascii_uppercase();
                let rest: String = chars.collect();
                format!("{first}{rest}")
            })
            .collect::<Vec<_>>()
            .join("");
    }
    raw.to_string()
}

#[derive(Default)]
struct NameSequence {
    counters: BTreeMap<String, usize>,
}

impl NameSequence {
    fn next(&mut self, prefix: &str) -> String {
        let counter = self.counters.entry(prefix.to_string()).or_insert(0);
        *counter += 1;
        let suffix = sequence_suffix(*counter);
        if !prefix.contains('_') && prefix.chars().any(|c| c.is_ascii_uppercase()) {
            format!("{prefix}{suffix}")
        } else {
            format!("{prefix}_{suffix}")
        }
    }
}

fn sequence_suffix(n: usize) -> String {
    let letter = ((n - 1) % 26) as u8;
    let number = ((n - 1) / 26) + 1;
    format!("{}{}", (b'A' + letter) as char, number)
}

fn collect_identifier_kinds(files: &[FileEntry]) -> BTreeMap<String, IdentifierKind> {
    let mut kinds = BTreeMap::new();
    let py_class_re = Regex::new(r"\bclass\s+([A-Za-z_][A-Za-z0-9_]*)").expect("regex");
    let py_def_re = Regex::new(r"\bdef\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("regex");
    let py_field_re = Regex::new(r"\bself\.([A-Za-z_][A-Za-z0-9_]*)").expect("regex");
    let sql_schema_table_re =
        Regex::new(r"(?i)\b([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)\b").expect("regex");
    let sql_table_kw_re =
        Regex::new(r"(?i)\b(from|join|update|into|table)\s+([A-Za-z_][A-Za-z0-9_]*)")
            .expect("regex");
    let ident_re = Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*\b").expect("regex");

    for file in files {
        let lang = detect_language(&file.rel, &file.text);
        match lang {
            Language::Python => {
                for cap in py_class_re.captures_iter(&file.text) {
                    kinds.insert(cap[1].to_string(), IdentifierKind::PyClass);
                }
                for cap in py_def_re.captures_iter(&file.text) {
                    kinds
                        .entry(cap[1].to_string())
                        .or_insert(IdentifierKind::PyMethod);
                }
                for cap in py_field_re.captures_iter(&file.text) {
                    kinds
                        .entry(cap[1].to_string())
                        .or_insert(IdentifierKind::PyField);
                }
                for ident in ident_re.find_iter(&file.text).map(|m| m.as_str()) {
                    if ident.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                        kinds
                            .entry(ident.to_string())
                            .or_insert(IdentifierKind::PyConst);
                    } else {
                        kinds
                            .entry(ident.to_string())
                            .or_insert(IdentifierKind::PyVar);
                    }
                }
            }
            Language::Sql => {
                for cap in sql_schema_table_re.captures_iter(&file.text) {
                    kinds
                        .entry(cap[1].to_string())
                        .or_insert(IdentifierKind::SqlSchema);
                    kinds
                        .entry(cap[2].to_string())
                        .or_insert(IdentifierKind::SqlTable);
                }
                for cap in sql_table_kw_re.captures_iter(&file.text) {
                    kinds
                        .entry(cap[2].to_string())
                        .or_insert(IdentifierKind::SqlTable);
                }
            }
            Language::Go => {
                for ident in ident_re.find_iter(&file.text).map(|m| m.as_str()) {
                    kinds
                        .entry(ident.to_string())
                        .or_insert(IdentifierKind::GoVar);
                }
            }
            _ => {}
        }
    }

    kinds
}

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
        assert!(!terms.contains("antifraud_check"));
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
    fn keeps_strings_and_comments_tokens_for_obfuscation() {
        let terms = detect_terms(&[FileEntry {
            rel: "main.py".into(),
            text: "# CustomerName comment\ntext = \"CustomerName in string\"\n".into(),
        }])
        .expect("terms");
        assert!(terms.contains("CustomerName"));
        assert!(terms.contains("comment"));
    }

    #[test]
    fn skips_magic_and_reserved_identifiers() {
        let terms = detect_terms(&[FileEntry {
            rel: "main.py".into(),
            text: "if __name__ == \"__main__\":\n    def main(self, cls):\n        return cls\n"
                .into(),
        }])
        .expect("terms");
        assert!(!terms.contains("__name__"));
        assert!(!terms.contains("__main__"));
        assert!(!terms.contains("main"));
        assert!(!terms.contains("self"));
        assert!(!terms.contains("cls"));
    }

    #[test]
    fn mapping_is_reversible_after_enrich() {
        let mut map = BTreeMap::new();
        map.insert("Alpha".to_string(), "Go".to_string());
        let terms = BTreeSet::from(["Beta".to_string(), "Gamma".to_string()]);
        let files: Vec<FileEntry> = Vec::new();
        enrich_with_random(&mut map, &terms, &files, Some(42));

        let reverse = invert(&map).expect("invert");
        for (from, to) in &map {
            assert_eq!(reverse.get(to), Some(from));
        }
    }

    #[test]
    fn avoids_namespace_collisions_and_entrypoints() {
        let files = vec![FileEntry {
            rel: "src/main.rs".into(),
            text: "fn main() { let Falcon1000 = 1; let Token = 2; }".into(),
        }];
        let mut map = BTreeMap::new();
        let mut terms = BTreeSet::new();
        terms.insert("Token".to_string());
        enrich_with_random(&mut map, &terms, &files, Some(1));
        let generated = map.get("Token").expect("mapped");
        assert_ne!(generated, "main");
        assert_ne!(generated, "Falcon1000");
    }

    #[test]
    fn does_not_collect_python_function_parameters() {
        let terms = detect_terms(&[FileEntry {
            rel: "main.py".into(),
            text: "def greet(user_name, send_to_folex=False):\n    return user_name\n".into(),
        }])
        .expect("terms");

        assert!(!terms.contains("user_name"));
        assert!(!terms.contains("send_to_folex"));
        assert!(terms.contains("greet"));
    }

    #[test]
    fn generates_kind_aware_prefixes() {
        let files = vec![
            FileEntry {
                rel: "main.py".into(),
                text: "class User:\n    def get_cats(self):\n        self.tag = 1\n\nCONST_A = 1\n"
                    .into(),
            },
            FileEntry {
                rel: "main.sql".into(),
                text: "SELECT * FROM my_schema.refill\n".into(),
            },
            FileEntry {
                rel: "main.go".into(),
                text: "package main\nfunc run() { var cards int }\n".into(),
            },
        ];

        let mut map = BTreeMap::new();
        let terms = BTreeSet::from([
            "User".to_string(),
            "get_cats".to_string(),
            "tag".to_string(),
            "CONST_A".to_string(),
            "my_schema".to_string(),
            "refill".to_string(),
            "cards".to_string(),
        ]);
        enrich_with_random(&mut map, &terms, &files, None);

        assert!(map.get("User").expect("User").starts_with("PyClass"));
        assert!(
            map.get("get_cats")
                .expect("get_cats")
                .starts_with("py_method_")
        );
        assert!(map.get("tag").expect("tag").starts_with("py_field_"));
        assert!(
            map.get("CONST_A")
                .expect("CONST_A")
                .starts_with("PY_CONST_")
        );
        assert!(
            map.get("my_schema")
                .expect("my_schema")
                .starts_with("sql_schema_")
        );
        assert!(map.get("refill").expect("refill").starts_with("sql_table_"));
        assert!(map.get("cards").expect("cards").starts_with("go_var_"));
    }
}
