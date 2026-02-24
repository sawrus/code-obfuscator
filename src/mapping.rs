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
    for m in re.find_iter(text) {
        let s = m.as_str();
        if !is_keyword(lang, s) {
            out.insert(s.to_string());
        }
    }
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
    fn keeps_strings_and_comments_tokens_for_obfuscation() {
        let terms = detect_terms(&[FileEntry {
            rel: "main.py".into(),
            text: "# CustomerName comment\ntext = \"CustomerName in string\"\n".into(),
        }])
        .expect("terms");
        assert!(terms.contains("CustomerName"));
        assert!(terms.contains("comment"));
        assert!(terms.contains("string"));
    }
}
