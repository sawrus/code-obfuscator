use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::AppResult;
use crate::fs_ops::FileEntry;
use crate::language::{Language, detect_language, is_keyword, is_protected_identifier};

pub fn transform_files(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    transform_files_with_options(files, map, true)
}

pub fn transform_files_with_options(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
    protect_public_api: bool,
) -> AppResult<Vec<(PathBuf, String)>> {
    Ok(files
        .iter()
        .map(|f| {
            let lang = detect_language(&f.rel, &f.text);
            (
                f.rel.clone(),
                transform_text(&f.text, lang, map, protect_public_api),
            )
        })
        .collect())
}

fn transform_text(
    text: &str,
    lang: Language,
    map: &BTreeMap<String, String>,
    protect_public_api: bool,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut current = String::new();
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut string_delim: Option<char> = None;
    let mut escape = false;

    while let Some(ch) = chars.next() {
        if let Some(delim) = string_delim {
            if escape {
                out.push(ch);
                escape = false;
                continue;
            }
            if ch == '\\' {
                out.push(ch);
                escape = true;
                continue;
            }
            if ch == delim {
                flush_ident(&mut current, &mut out, lang, map, false, protect_public_api);
                out.push(ch);
                string_delim = None;
                continue;
            }
            if is_ident_char(ch) {
                current.push(ch);
            } else {
                flush_ident(&mut current, &mut out, lang, map, false, protect_public_api);
                out.push(ch);
            }
            continue;
        }

        if in_line_comment {
            if ch == '\n' {
                flush_ident(&mut current, &mut out, lang, map, false, protect_public_api);
                in_line_comment = false;
                out.push(ch);
            } else if is_ident_char(ch) {
                current.push(ch);
            } else {
                flush_ident(&mut current, &mut out, lang, map, false, protect_public_api);
                out.push(ch);
            }
            continue;
        }

        if in_block_comment {
            if ch == '*' && chars.peek().copied() == Some('/') {
                flush_ident(&mut current, &mut out, lang, map, false, protect_public_api);
                out.push(ch);
                out.push(chars.next().expect("peeked"));
                in_block_comment = false;
            } else if is_ident_char(ch) {
                current.push(ch);
            } else {
                flush_ident(&mut current, &mut out, lang, map, false, protect_public_api);
                out.push(ch);
            }
            continue;
        }

        if starts_line_comment(lang, ch, chars.peek().copied()) {
            flush_ident(&mut current, &mut out, lang, map, true, protect_public_api);
            out.push(ch);
            out.push(chars.next().expect("peeked"));
            in_line_comment = true;
            continue;
        }

        if starts_block_comment(lang, ch, chars.peek().copied()) {
            flush_ident(&mut current, &mut out, lang, map, true, protect_public_api);
            out.push(ch);
            out.push(chars.next().expect("peeked"));
            in_block_comment = true;
            continue;
        }

        if is_comment_start_hash(lang, ch) {
            flush_ident(&mut current, &mut out, lang, map, true, protect_public_api);
            out.push(ch);
            in_line_comment = true;
            continue;
        }

        if is_string_delim(lang, ch) {
            flush_ident(&mut current, &mut out, lang, map, true, protect_public_api);
            out.push(ch);
            string_delim = Some(ch);
            continue;
        }

        if is_ident_char(ch) {
            current.push(ch);
        } else {
            flush_ident(&mut current, &mut out, lang, map, true, protect_public_api);
            out.push(ch);
        }
    }

    flush_ident(&mut current, &mut out, lang, map, true, protect_public_api);
    out
}

fn flush_ident(
    current: &mut String,
    out: &mut String,
    lang: Language,
    map: &BTreeMap<String, String>,
    strict_code_context: bool,
    protect_public_api: bool,
) {
    if current.is_empty() {
        return;
    }
    let token = std::mem::take(current);
    if !protect_public_api && let Some(to) = map.get(&token) {
        out.push_str(to);
        return;
    }

    let protected = is_keyword(lang, &token)
        || is_protected_identifier(lang, &token)
        || (strict_code_context && protect_public_api && looks_like_public_api(lang, &token));

    if protected {
        out.push_str(&token);
    } else if let Some(to) = map.get(&token) {
        out.push_str(to);
    } else {
        out.push_str(&token);
    }
}

fn looks_like_public_api(lang: Language, token: &str) -> bool {
    match lang {
        Language::JavaScript | Language::TypeScript | Language::Java | Language::CSharp => {
            token.chars().next().is_some_and(char::is_uppercase)
        }
        _ => false,
    }
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn starts_line_comment(lang: Language, ch: char, peek: Option<char>) -> bool {
    matches!(
        lang,
        Language::JavaScript
            | Language::TypeScript
            | Language::Java
            | Language::CSharp
            | Language::CCpp
            | Language::Go
            | Language::Rust
    ) && ch == '/'
        && peek == Some('/')
}

fn starts_block_comment(lang: Language, ch: char, peek: Option<char>) -> bool {
    matches!(
        lang,
        Language::JavaScript
            | Language::TypeScript
            | Language::Java
            | Language::CSharp
            | Language::CCpp
            | Language::Go
            | Language::Rust
            | Language::Sql
    ) && ch == '/'
        && peek == Some('*')
}

fn is_comment_start_hash(lang: Language, ch: char) -> bool {
    matches!(lang, Language::Python | Language::Bash) && ch == '#'
}

fn is_string_delim(lang: Language, ch: char) -> bool {
    match lang {
        Language::Unknown => false,
        _ => ch == '"' || ch == '\'',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_python_dunder_runtime_markers() {
        let mut map = BTreeMap::new();
        map.insert("__name__".into(), "BadName".into());
        map.insert("__main__".into(), "BadMain".into());
        map.insert("run_workers".into(), "Run".into());
        let f = vec![FileEntry {
            rel: "a.py".into(),
            text: "if __name__ == \"__main__\":\n  run_workers()".into(),
        }];
        let out = transform_files(&f, &map).expect("transform");
        assert!(out[0].1.contains("__name__"));
        assert!(out[0].1.contains("__main__"));
        assert!(out[0].1.contains("Run()"));
    }

    #[test]
    fn obfuscates_comment_and_string_identifiers() {
        let mut map = BTreeMap::new();
        map.insert("Freeze".into(), "Go".into());
        let f = vec![FileEntry {
            rel: "a.rs".into(),
            text: "// Freeze\nlet x = \"Freeze\";".into(),
        }];
        let out = transform_files(&f, &map).expect("transform");
        assert_eq!(out[0].1, "// Go\nlet x = \"Go\";");
    }
}
