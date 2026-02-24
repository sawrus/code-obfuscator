use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::AppResult;
use crate::fs_ops::FileEntry;
use crate::language::{Language, detect_language};

pub fn transform_files(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    Ok(files
        .iter()
        .map(|f| {
            let lang = detect_language(&f.rel, &f.text);
            (f.rel.clone(), apply_rules(&f.text, map, lang))
        })
        .collect())
}

fn apply_rules(text: &str, map: &BTreeMap<String, String>, lang: Language) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if let Some((next, segment)) = scan_string_or_comment(text, i, lang) {
            out.push_str(segment);
            i = next;
            continue;
        }

        let ch = text[i..].chars().next().expect("char");
        if is_ident_start(ch) {
            let start = i;
            i += ch.len_utf8();
            while i < text.len() {
                let c = text[i..].chars().next().expect("char");
                if !is_ident_continue(c) {
                    break;
                }
                i += c.len_utf8();
            }
            let token = &text[start..i];
            if is_reserved_identifier(token)
                || is_python_snake_case_identifier(token, lang)
                || is_named_arg_label(text, start, i, lang)
                || is_python_import_path_token(text, start, i, lang)
            {
                out.push_str(token);
            } else {
                out.push_str(map.get(token).map(|s| s.as_str()).unwrap_or(token));
            }
            continue;
        }

        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn scan_string_or_comment(text: &str, i: usize, lang: Language) -> Option<(usize, &str)> {
    let rest = &text[i..];
    if supports_hash_comments(lang) && rest.starts_with('#') {
        let end = rest.find('\n').map(|x| i + x).unwrap_or(text.len());
        return Some((end, &text[i..end]));
    }
    if supports_line_comments(lang) && rest.starts_with("//") {
        let end = rest.find('\n').map(|x| i + x).unwrap_or(text.len());
        return Some((end, &text[i..end]));
    }
    if supports_block_comments(lang) && rest.starts_with("/*") {
        let end = rest.find("*/").map(|x| i + x + 2).unwrap_or(text.len());
        return Some((end, &text[i..end]));
    }
    if supports_sql_comments(lang) && rest.starts_with("--") {
        let end = rest.find('\n').map(|x| i + x).unwrap_or(text.len());
        return Some((end, &text[i..end]));
    }
    if rest.starts_with('"') || rest.starts_with('\'') || rest.starts_with('`') {
        let quote = rest.chars().next().expect("quote");
        let mut j = i + quote.len_utf8();
        let mut escaped = false;
        while j < text.len() {
            let c = text[j..].chars().next().expect("char");
            j += c.len_utf8();
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == quote {
                break;
            }
        }
        return Some((j, &text[i..j]));
    }
    None
}

fn supports_hash_comments(lang: Language) -> bool {
    matches!(lang, Language::Python | Language::Bash)
}

fn supports_line_comments(lang: Language) -> bool {
    matches!(
        lang,
        Language::JavaScript
            | Language::TypeScript
            | Language::Java
            | Language::CSharp
            | Language::CCpp
            | Language::Go
            | Language::Rust
    )
}

fn supports_block_comments(lang: Language) -> bool {
    supports_line_comments(lang) || matches!(lang, Language::Sql)
}

fn supports_sql_comments(lang: Language) -> bool {
    matches!(lang, Language::Sql)
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

fn is_reserved_identifier(token: &str) -> bool {
    (token.starts_with("__") && token.ends_with("__"))
        || matches!(
            token,
            "print"
                | "console"
                | "log"
                | "System"
                | "out"
                | "println"
                | "Println"
                | "Console"
                | "WriteLine"
                | "fmt"
                | "std"
                | "cout"
                | "endl"
                | "Main"
                | "Program"
                | "String"
                | "args"
                | "include"
                | "iostream"
                | "string"
                | "str"
                | "echo"
        )
}

fn is_python_snake_case_identifier(token: &str, lang: Language) -> bool {
    matches!(lang, Language::Python) && token.contains('_')
}

fn is_python_import_path_token(text: &str, start: usize, end: usize, lang: Language) -> bool {
    if !matches!(lang, Language::Python) {
        return false;
    }
    let line_start = text[..start].rfind('\n').map(|x| x + 1).unwrap_or(0);
    let line_end = text[end..]
        .find('\n')
        .map(|x| end + x)
        .unwrap_or(text.len());
    let line = &text[line_start..line_end];
    let trimmed = line.trim_start();

    if let Some(import_idx) = trimmed.find(" import ") {
        if trimmed.starts_with("from ") {
            let module_start = line_start + (line.len() - trimmed.len()) + "from ".len();
            let module_end = line_start + (line.len() - trimmed.len()) + import_idx;
            return start >= module_start && end <= module_end;
        }
    }
    if trimmed.starts_with("import ") {
        return true;
    }
    false
}

fn is_named_arg_label(text: &str, start: usize, end: usize, lang: Language) -> bool {
    if !matches!(lang, Language::Python) {
        return false;
    }
    let mut j = end;
    while j < text.len() {
        let c = text[j..].chars().next().expect("char");
        if c.is_whitespace() {
            j += c.len_utf8();
            continue;
        }
        if c != '=' {
            return false;
        }
        break;
    }
    if j >= text.len() {
        return false;
    }
    let left = text[..start].chars().rev().find(|c| !c.is_whitespace());
    matches!(left, Some('(' | ','))
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

    #[test]
    fn does_not_replace_strings_comments_or_magic_names() {
        let mut map = BTreeMap::new();
        map.insert("module_path".into(), "Shadow".into());
        map.insert("run_task".into(), "Launch".into());
        map.insert("__name__".into(), "Broken".into());
        let f = vec![FileEntry {
            rel: "main.py".into(),
            text: "if __name__ == \"__main__\":\n    from module_path import run_task\n    run_task(module_path=\"module_path\")\n".into(),
        }];
        let out = transform_files(&f, &map).expect("transform");
        assert!(
            out[0]
                .1
                .contains("if __name__ == \"__main__\":\n    from module_path import")
        );
        assert!(out[0].1.contains("run_task(module_path=\"module_path\")"));
    }
}
