use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::error::AppResult;
use crate::fs_ops::FileEntry;
use crate::language::{Language, detect_language};

pub fn transform_files(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    let globally_imported_python_symbols = collect_python_imported_symbols_from_files(files);
    Ok(files
        .iter()
        .map(|f| {
            let lang = detect_language(&f.rel, &f.text);
            (
                f.rel.clone(),
                apply_rules(&f.text, map, lang, &globally_imported_python_symbols),
            )
        })
        .collect())
}

fn apply_rules(
    text: &str,
    map: &BTreeMap<String, String>,
    lang: Language,
    globally_imported_python_symbols: &BTreeSet<String>,
) -> String {
    let python_imports = if matches!(lang, Language::Python) {
        collect_python_imported_symbols(text)
    } else {
        BTreeSet::new()
    };
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
                || is_python_builtin_identifier(token, lang)
                || python_imports.contains(token)
                || globally_imported_python_symbols.contains(token)
                || is_javascript_camel_case_identifier(token, lang)
                || is_python_import_line(text, start, lang)
                || is_python_import_path_token(text, start, i, lang)
                || is_member_access_identifier(text, start, i, lang)
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
                | "main"
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

fn is_python_builtin_identifier(token: &str, lang: Language) -> bool {
    matches!(lang, Language::Python) && PYTHON_BUILTINS.contains(&token)
}

fn is_javascript_camel_case_identifier(token: &str, lang: Language) -> bool {
    if !matches!(lang, Language::JavaScript | Language::TypeScript) {
        return false;
    }
    token.chars().any(|c| c.is_ascii_uppercase())
}

fn is_member_access_identifier(text: &str, start: usize, end: usize, lang: Language) -> bool {
    if matches!(lang, Language::Sql) {
        return false;
    }
    let prev = text[..start].chars().rev().find(|c| !c.is_whitespace());
    let rest = text[end..].trim_start();
    matches!(prev, Some('.' | ':' | '#')) || rest.starts_with('.') || rest.starts_with("::")
}

fn collect_python_imported_symbols_from_files(files: &[FileEntry]) -> BTreeSet<String> {
    files
        .iter()
        .filter(|f| matches!(detect_language(&f.rel, &f.text), Language::Python))
        .flat_map(|f| collect_python_imported_symbols(&f.text).into_iter())
        .collect()
}

fn collect_python_imported_symbols(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            for chunk in rest.split(',') {
                let symbol = chunk.trim().split(" as ").next().unwrap_or_default();
                if let Some(last) = symbol.rsplit('.').next()
                    && !last.is_empty()
                {
                    out.insert(last.to_string());
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("from ")
            && let Some(imported) = rest.split(" import ").nth(1)
        {
            for part in imported.split(',') {
                let symbol = part.trim().split(" as ").next().unwrap_or_default();
                if symbol != "*" && !symbol.is_empty() {
                    out.insert(symbol.to_string());
                }
            }
        }
    }
    out
}

const PYTHON_BUILTINS: &[&str] = &[
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "BytesWarning",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "Ellipsis",
    "EnvironmentError",
    "Exception",
    "False",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "IOError",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "None",
    "NotADirectoryError",
    "NotImplemented",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "True",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "ZeroDivisionError",
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "copyright",
    "credits",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "exit",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "license",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "quit",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__build_class__",
    "__debug__",
    "__doc__",
    "__import__",
    "__loader__",
    "__name__",
    "__package__",
    "__spec__",
];

fn is_python_import_line(text: &str, start: usize, lang: Language) -> bool {
    if !matches!(lang, Language::Python) {
        return false;
    }
    let line_start = text[..start].rfind('\n').map(|x| x + 1).unwrap_or(0);
    let line_end = text[start..]
        .find('\n')
        .map(|x| start + x)
        .unwrap_or(text.len());
    let line = &text[line_start..line_end];
    let trimmed = line.trim_start();
    trimmed.starts_with("from ") || trimmed.starts_with("import ")
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

    if let Some(import_idx) = trimmed.find(" import ")
        && trimmed.starts_with("from ")
    {
        let module_start = line_start + (line.len() - trimmed.len()) + "from ".len();
        let module_end = line_start + (line.len() - trimmed.len()) + import_idx;
        return start >= module_start && end <= module_end;
    }
    if trimmed.starts_with("import ") {
        return true;
    }
    false
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
        assert!(out[0].1.contains("run_task("));
        assert!(out[0].1.contains("=\"module_path\")"));
    }

    #[test]
    fn python_replaces_snake_case_and_methods_but_keeps_imported_classes() {
        let mut map = BTreeMap::new();
        map.insert(
            "get_suspect_users_from_refill_actions".into(),
            "get_a_b_c".into(),
        );
        map.insert("PG_MWL_PASSWORD".into(), "PG_CAT_P".into());
        map.insert("User".into(), "Amber2096".into());

        let f = vec![FileEntry {
            rel: "service.py".into(),
            text: "from apiutil.models import User\n\nPG_MWL_PASSWORD = \"x\"\n\ndef get_suspect_users_from_refill_actions():\n    return PG_MWL_PASSWORD\n\nclass Falcon8382(User):\n    pass\n"
                .into(),
        }];

        let out = transform_files(&f, &map).expect("transform");
        assert!(out[0].1.contains("PG_CAT_P = \"x\""));
        assert!(out[0].1.contains("def get_a_b_c():"));
        assert!(out[0].1.contains("return PG_CAT_P"));
        assert!(out[0].1.contains("class Falcon8382(User):"));
    }

    #[test]
    fn sql_replaces_qualified_column_names() {
        let mut map = BTreeMap::new();
        map.insert("refill".into(), "test666".into());
        map.insert("user_id".into(), "a1".into());
        map.insert("amount".into(), "b1".into());
        map.insert("code".into(), "c1".into());

        let f = vec![FileEntry {
            rel: "q.sql".into(),
            text: "SELECT r.user_id, amount, code FROM refill r WHERE r.user_id > 0".into(),
        }];
        let out = transform_files(&f, &map).expect("transform");
        assert_eq!(
            out[0].1,
            "SELECT r.a1, b1, c1 FROM test666 r WHERE r.a1 > 0"
        );
    }
}
