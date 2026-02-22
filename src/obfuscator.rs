use std::collections::BTreeMap;
use std::path::PathBuf;

use regex::Regex;

use crate::error::AppResult;
use crate::fs_ops::FileEntry;

pub fn transform_files(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    let rules = compile_rules(map)?;
    Ok(files
        .iter()
        .map(|f| (f.rel.clone(), apply_rules(&f.text, &rules)))
        .collect())
}

fn compile_rules(map: &BTreeMap<String, String>) -> AppResult<Vec<(Regex, String)>> {
    let mut pairs: Vec<_> = map.iter().collect();
    pairs.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    let mut out = Vec::new();
    for (from, to) in pairs {
        out.push((word_re(from)?, to.clone()));
    }
    Ok(out)
}

fn word_re(word: &str) -> AppResult<Regex> {
    let esc = regex::escape(word);
    Ok(Regex::new(&format!(r"\b{}\b", esc))?)
}

fn apply_rules(text: &str, rules: &[(Regex, String)]) -> String {
    rules.iter().fold(text.to_string(), |acc, (re, to)| {
        re.replace_all(&acc, to.as_str()).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_whole_words() {
        let mut map = BTreeMap::new();
        map.insert("Freeze".into(), "Go".into());
        let f = vec![FileEntry {
            rel: "a.rs".into(),
            text: "Freeze Freezer".into(),
        }];
        let out = transform_files(&f, &map).expect("transform");
        assert_eq!(out[0].1, "Go Freezer");
    }
}
