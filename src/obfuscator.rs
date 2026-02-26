use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::error::AppResult;
use crate::fs_ops::FileEntry;
use crate::language::{Language, detect_language, is_keyword};

pub fn transform_files(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    let globally_imported_python_symbols =
        collect_python_external_imported_symbols_from_files(files);
    Ok(files
        .iter()
        .map(|f| {
            let lang = detect_language(&f.rel, &f.text);
            let obfuscator = build_obfuscator(lang);
            (
                f.rel.clone(),
                obfuscator.apply(&f.text, map, &globally_imported_python_symbols),
            )
        })
        .collect())
}

trait LanguageObfuscator {
    fn apply(
        &self,
        text: &str,
        map: &BTreeMap<String, String>,
        globally_imported_python_symbols: &BTreeSet<String>,
    ) -> String;
}

fn build_obfuscator(lang: Language) -> Box<dyn LanguageObfuscator> {
    match lang {
        Language::Python => Box::new(PythonObfuscator),
        Language::Sql => Box::new(SqlObfuscator),
        Language::JavaScript => Box::new(JavaScriptObfuscator),
        Language::TypeScript => Box::new(TypeScriptObfuscator),
        Language::Java => Box::new(JavaObfuscator),
        Language::CSharp => Box::new(CSharpObfuscator),
        Language::CCpp => Box::new(CCppObfuscator),
        Language::Go => Box::new(GoObfuscator),
        Language::Rust => Box::new(RustObfuscator),
        Language::Bash => Box::new(BashObfuscator),
        Language::Unknown => Box::new(UnknownObfuscator),
    }
}

struct PythonObfuscator;
struct SqlObfuscator;
struct JavaScriptObfuscator;
struct TypeScriptObfuscator;
struct JavaObfuscator;
struct CSharpObfuscator;
struct CCppObfuscator;
struct GoObfuscator;
struct RustObfuscator;
struct BashObfuscator;
struct UnknownObfuscator;

macro_rules! impl_lang_obfuscator {
    ($name:ident, $lang:expr) => {
        impl LanguageObfuscator for $name {
            fn apply(
                &self,
                text: &str,
                map: &BTreeMap<String, String>,
                globally_imported_python_symbols: &BTreeSet<String>,
            ) -> String {
                apply_rules(text, map, $lang, globally_imported_python_symbols)
            }
        }
    };
}

impl_lang_obfuscator!(PythonObfuscator, Language::Python);
impl_lang_obfuscator!(SqlObfuscator, Language::Sql);
impl_lang_obfuscator!(JavaScriptObfuscator, Language::JavaScript);
impl_lang_obfuscator!(TypeScriptObfuscator, Language::TypeScript);
impl_lang_obfuscator!(JavaObfuscator, Language::Java);
impl_lang_obfuscator!(CSharpObfuscator, Language::CSharp);
impl_lang_obfuscator!(CCppObfuscator, Language::CCpp);
impl_lang_obfuscator!(GoObfuscator, Language::Go);
impl_lang_obfuscator!(RustObfuscator, Language::Rust);
impl_lang_obfuscator!(BashObfuscator, Language::Bash);
impl_lang_obfuscator!(UnknownObfuscator, Language::Unknown);

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
        if let Some((next, segment, kind)) = scan_string_or_comment(text, i, lang) {
            let transformed_segment = if matches!(kind, SegmentKind::String) {
                maybe_obfuscate_sql_in_string(segment, map, lang)
            } else {
                segment.to_string()
            };
            out.push_str(&replace_mapped_identifiers(&transformed_segment, map));
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
            if let Some(mapped) = map.get(token)
                && !is_reserved_identifier(token)
                && !is_keyword(lang, token)
                && !is_sql_built_in_function(token, lang)
                && !is_python_builtin_identifier(token, lang)
                && !globally_imported_python_symbols.contains(token)
                && !is_python_import_path_token(text, start, i, lang)
                && !is_member_access_identifier(text, start, i, lang)
            {
                out.push_str(mapped);
                continue;
            }

            if is_reserved_identifier(token)
                || is_keyword(lang, token)
                || is_sql_built_in_function(token, lang)
                || is_python_builtin_identifier(token, lang)
                || is_python_def_parameter(text, start, lang)
                || is_python_keyword_argument_label(text, start, i, lang)
                || python_imports.contains(token)
                || globally_imported_python_symbols.contains(token)
                || is_javascript_camel_case_identifier(token, lang)
                || is_python_import_line(text, start, lang)
                || is_python_import_path_token(text, start, i, lang)
                || is_non_python_import_line(text, start, lang)
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

fn replace_mapped_identifiers(text: &str, map: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < text.len() {
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
            out.push_str(map.get(token).map(|s| s.as_str()).unwrap_or(token));
            continue;
        }

        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

#[derive(Copy, Clone)]
enum SegmentKind {
    Comment,
    String,
}

fn scan_string_or_comment(
    text: &str,
    i: usize,
    lang: Language,
) -> Option<(usize, &str, SegmentKind)> {
    let rest = &text[i..];

    if matches!(lang, Language::Python)
        && let Some((prefix_len, quote_len, quote)) = python_string_prefix(rest)
    {
        let mut j = i + prefix_len + quote_len;
        let mut escaped = false;
        while j < text.len() {
            let c = text[j..].chars().next().expect("char");
            if quote_len == 3 {
                if text[j..].starts_with(quote) {
                    j += 3;
                    break;
                }
                j += c.len_utf8();
                continue;
            }

            j += c.len_utf8();
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == quote.chars().next().expect("quote") {
                break;
            }
        }
        return Some((j, &text[i..j], SegmentKind::String));
    }

    if supports_hash_comments(lang) && rest.starts_with('#') {
        let end = rest.find('\n').map(|x| i + x).unwrap_or(text.len());
        return Some((end, &text[i..end], SegmentKind::Comment));
    }
    if supports_line_comments(lang) && rest.starts_with("//") {
        let end = rest.find('\n').map(|x| i + x).unwrap_or(text.len());
        return Some((end, &text[i..end], SegmentKind::Comment));
    }
    if supports_block_comments(lang) && rest.starts_with("/*") {
        let end = rest.find("*/").map(|x| i + x + 2).unwrap_or(text.len());
        return Some((end, &text[i..end], SegmentKind::Comment));
    }
    if supports_sql_comments(lang) && rest.starts_with("--") {
        let end = rest.find('\n').map(|x| i + x).unwrap_or(text.len());
        return Some((end, &text[i..end], SegmentKind::Comment));
    }
    if matches!(lang, Language::Python) && (rest.starts_with("\"\"\"") || rest.starts_with("'''")) {
        let quote = &rest[..3];
        let end = rest
            .get(3..)
            .and_then(|tail| tail.find(quote).map(|x| i + 3 + x + 3))
            .unwrap_or(text.len());
        return Some((end, &text[i..end], SegmentKind::String));
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
        return Some((j, &text[i..j], SegmentKind::String));
    }
    None
}

fn maybe_obfuscate_sql_in_string(
    segment: &str,
    map: &BTreeMap<String, String>,
    lang: Language,
) -> String {
    if matches!(lang, Language::Sql) {
        return segment.to_string();
    }

    let Some((prefix_len, quote_len, is_fstring)) = string_parts(segment, lang) else {
        return segment.to_string();
    };

    let content = &segment[prefix_len + quote_len..segment.len() - quote_len];
    let fstring_content = if is_fstring {
        obfuscate_python_fstring_expressions(content, map)
    } else {
        content.to_string()
    };

    if !looks_like_sql(&fstring_content) {
        if is_fstring {
            return rebuild_string(segment, prefix_len, quote_len, &fstring_content);
        }
        return segment.to_string();
    }

    let obfuscated = apply_rules(&fstring_content, map, Language::Sql, &BTreeSet::new());
    rebuild_string(segment, prefix_len, quote_len, &obfuscated)
}

fn rebuild_string(segment: &str, prefix_len: usize, quote_len: usize, content: &str) -> String {
    let mut out =
        String::with_capacity(segment.len() + content.len().saturating_sub(segment.len()));
    out.push_str(&segment[..prefix_len + quote_len]);
    out.push_str(content);
    out.push_str(&segment[segment.len() - quote_len..]);
    out
}

fn string_parts(segment: &str, lang: Language) -> Option<(usize, usize, bool)> {
    if matches!(lang, Language::Python)
        && let Some((prefix_len, quote_len, _)) = python_string_prefix(segment)
    {
        let prefix = &segment[..prefix_len];
        return Some((
            prefix_len,
            quote_len,
            prefix.chars().any(|c| matches!(c, 'f' | 'F')),
        ));
    }

    let is_triple_quoted = ((segment.starts_with("\"\"\"") && segment.ends_with("\"\"\""))
        || (segment.starts_with("'''") && segment.ends_with("'''")))
        && segment.len() >= 6;
    if is_triple_quoted {
        return Some((0, 3, false));
    }

    if (segment.starts_with('"') && segment.ends_with('"'))
        || (segment.starts_with('\'') && segment.ends_with('\''))
        || (segment.starts_with('`') && segment.ends_with('`'))
    {
        return Some((0, 1, false));
    }

    None
}

fn python_string_prefix(rest: &str) -> Option<(usize, usize, &'static str)> {
    const PREFIXES: &[&str] = &[
        "", "r", "u", "b", "f", "R", "U", "B", "F", "fr", "rf", "Fr", "fR", "RF", "rF", "FR", "br",
        "rb", "Br", "bR", "RB", "rB", "BR",
    ];

    for prefix in PREFIXES {
        if rest.len() < prefix.len() {
            continue;
        }
        let candidate = &rest[prefix.len()..];
        for quote in ["\"\"\"", "'''", "\"", "'"] {
            if candidate.starts_with(quote) {
                return Some((prefix.len(), quote.len(), quote));
            }
        }
    }
    None
}

fn obfuscate_python_fstring_expressions(content: &str, map: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '{' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                out.push('{');
                out.push('{');
                i += 2;
                continue;
            }
            let mut j = i + 1;
            let mut depth = 1;
            while j < chars.len() {
                if chars[j] == '{' {
                    depth += 1;
                } else if chars[j] == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }

            if j < chars.len() && chars[j] == '}' {
                let expr: String = chars[i + 1..j].iter().collect();
                let expr_obf = apply_rules(&expr, map, Language::Python, &BTreeSet::new());
                out.push('{');
                out.push_str(&expr_obf);
                out.push('}');
                i = j + 1;
                continue;
            }
        }

        out.push(ch);
        i += 1;
    }

    out
}

fn is_python_def_parameter(text: &str, start: usize, lang: Language) -> bool {
    if !matches!(lang, Language::Python) {
        return false;
    }
    let line_start = text[..start].rfind('\n').map_or(0, |idx| idx + 1);
    let line_end = text[start..]
        .find('\n')
        .map_or(text.len(), |idx| start + idx);
    let line = &text[line_start..line_end];

    let Some(def_pos) = line.find("def ") else {
        return false;
    };
    let Some(open_pos) = line[def_pos..].find('(').map(|x| def_pos + x) else {
        return false;
    };
    let Some(close_pos) = line[open_pos..].find(')').map(|x| open_pos + x) else {
        return false;
    };
    let abs_open = line_start + open_pos;
    let abs_close = line_start + close_pos;
    start > abs_open && start < abs_close
}

fn is_python_keyword_argument_label(text: &str, start: usize, end: usize, lang: Language) -> bool {
    if !matches!(lang, Language::Python) {
        return false;
    }

    let next = text[end..].chars().find(|c| !c.is_whitespace());
    if next != Some('=') {
        return false;
    }
    let second = text[end..]
        .chars()
        .filter(|c| !c.is_whitespace())
        .nth(1)
        .unwrap_or('\0');
    if second == '=' {
        return false;
    }

    let prev = text[..start].chars().rev().find(|c| !c.is_whitespace());
    matches!(prev, Some('(' | ','))
}

fn looks_like_sql(content: &str) -> bool {
    let normalized = content.to_ascii_lowercase();
    let tokens: BTreeSet<&str> = normalized
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .collect();

    (tokens.contains("select") && (tokens.contains("from") || tokens.contains("join")))
        || (tokens.contains("insert") && tokens.contains("into"))
        || tokens.contains("update")
        || (tokens.contains("delete") && tokens.contains("from"))
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
                | "new"
                | "Map"
                | "Set"
                | "List"
                | "ArrayList"
                | "HashMap"
                | "BTreeMap"
                | "Vec"
                | "Result"
                | "Option"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
                | "bool"
                | "char"
                | "fn"
                | "SELECT"
                | "FROM"
                | "WHERE"
                | "INSERT"
                | "UPDATE"
                | "DELETE"
                | "JOIN"
                | "CREATE"
                | "TABLE"
                | "VIEW"
                | "DISTINCT"
                | "ARRAY_AGG"
                | "COUNT"
                | "SUM"
                | "MIN"
                | "MAX"
                | "AVG"
                | "GROUP_CONCAT"
                | "COALESCE"
                | "NOW"
                | "CURRENT_DATE"
                | "CURRENT_TIMESTAMP"
        )
}

fn is_sql_built_in_function(token: &str, lang: Language) -> bool {
    if !matches!(lang, Language::Sql) {
        return false;
    }
    matches!(
        token.to_ascii_lowercase().as_str(),
        "distinct"
            | "array_agg"
            | "count"
            | "sum"
            | "min"
            | "max"
            | "avg"
            | "group_concat"
            | "coalesce"
            | "ifnull"
            | "nvl"
            | "date_trunc"
            | "to_start_of_month"
            | "now"
            | "current_date"
            | "current_timestamp"
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

fn is_member_access_identifier(text: &str, start: usize, _end: usize, lang: Language) -> bool {
    if matches!(lang, Language::Sql) {
        return false;
    }

    if matches!(lang, Language::Python) && is_python_local_member_access_identifier(text, start) {
        return false;
    }

    let mut prev = None;
    let mut prev_idx = 0usize;
    for (idx, c) in text[..start].char_indices().rev() {
        if c == '\n' || c == '\r' {
            break;
        }
        if c.is_whitespace() {
            continue;
        }
        prev = Some(c);
        prev_idx = idx;
        break;
    }

    match prev {
        Some('.') | Some('#') => true,
        Some(':') => text[..prev_idx].chars().rev().find(|c| !c.is_whitespace()) == Some(':'),
        _ => false,
    }
}

fn is_python_local_member_access_identifier(text: &str, start: usize) -> bool {
    if start == 0 {
        return false;
    }

    let prefix = &text[..start];
    let trimmed = prefix.trim_end();
    if !trimmed.ends_with('.') {
        return false;
    }

    let owner = trimmed[..trimmed.len() - 1]
        .trim_end()
        .rsplit(|c: char| !is_ident_continue(c))
        .next()
        .unwrap_or_default();

    if matches!(owner, "self" | "cls") {
        return true;
    }

    let expr_start = prefix
        .rfind(['\n', ';', '='])
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let expr = &prefix[expr_start..];
    expr.contains("self.") || expr.contains("cls.")
}

fn is_non_python_import_line(text: &str, start: usize, lang: Language) -> bool {
    if matches!(lang, Language::Python | Language::Sql | Language::Unknown) {
        return false;
    }

    let line_start = text[..start].rfind('\n').map_or(0, |idx| idx + 1);
    let line = text[line_start..]
        .lines()
        .next()
        .unwrap_or_default()
        .trim_start();

    match lang {
        Language::Rust => line.starts_with("use ") || line.starts_with("extern crate "),
        Language::Java | Language::CSharp => {
            line.starts_with("import ")
                || line.starts_with("package ")
                || line.starts_with("using ")
        }
        Language::CCpp => {
            line.starts_with("#include")
                || line.starts_with("using ")
                || line.starts_with("namespace ")
        }
        Language::JavaScript | Language::TypeScript => {
            line.starts_with("import ") || line.starts_with("export ")
        }
        Language::Go => line.starts_with("import ") || line.starts_with("package "),
        Language::Bash => false,
        Language::Python | Language::Sql | Language::Unknown => false,
    }
}

fn collect_python_external_imported_symbols_from_files(files: &[FileEntry]) -> BTreeSet<String> {
    let declared = collect_python_declared_symbols_from_files(files);
    let imported: BTreeSet<String> = files
        .iter()
        .filter(|f| matches!(detect_language(&f.rel, &f.text), Language::Python))
        .flat_map(|f| collect_python_imported_symbols(&f.text).into_iter())
        .collect();

    imported
        .into_iter()
        .filter(|symbol| !declared.contains(symbol))
        .collect()
}

fn collect_python_declared_symbols_from_files(files: &[FileEntry]) -> BTreeSet<String> {
    files
        .iter()
        .filter(|f| matches!(detect_language(&f.rel, &f.text), Language::Python))
        .flat_map(|f| collect_python_declared_symbols(&f.text).into_iter())
        .collect()
}

fn collect_python_declared_symbols(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let class_re = Regex::new(r"\bclass\s+([A-Za-z_][A-Za-z0-9_]*)").expect("regex");
    let def_re = Regex::new(r"\bdef\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("regex");

    out.extend(
        class_re
            .captures_iter(text)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string())),
    );
    out.extend(
        def_re
            .captures_iter(text)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string())),
    );
    out
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
    fn applies_mapping_inside_strings_and_comments_with_high_priority() {
        let mut map = BTreeMap::new();
        map.insert("Turkey".into(), "T1".into());
        map.insert("tag_name".into(), "tag_obf".into());
        let f = vec![FileEntry {
            rel: "main.py".into(),
            text: "# Tag \"Turkey\"\ntag_name = \"Turkey\"\n".into(),
        }];
        let out = transform_files(&f, &map).expect("transform");
        assert!(out[0].1.contains("# Tag \"T1\""));
        assert!(out[0].1.contains("tag_obf = \"T1\""));
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

    #[test]
    fn sql_keeps_keywords_and_common_aggregates() {
        let map = BTreeMap::from([
            ("distinct".to_string(), "broken1".to_string()),
            ("array_agg".to_string(), "broken2".to_string()),
            ("count".to_string(), "broken3".to_string()),
            ("orders".to_string(), "t_orders".to_string()),
            ("user_id".to_string(), "u1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.sql".into(),
            text: "SELECT DISTINCT user_id, ARRAY_AGG(user_id), COUNT(user_id) FROM orders".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("SELECT DISTINCT"));
        assert!(out[0].1.contains("ARRAY_AGG(u1)"));
        assert!(out[0].1.contains("COUNT(u1)"));
        assert!(out[0].1.contains("FROM t_orders"));
        assert!(!out[0].1.contains("broken1"));
        assert!(!out[0].1.contains("broken2"));
        assert!(!out[0].1.contains("broken3"));
    }

    #[test]
    fn deep_obfuscation_replacements_work_across_non_python_languages() {
        let mut map = BTreeMap::new();
        map.insert("refill_action".into(), "r1".into());
        map.insert("user_id".into(), "u1".into());

        let files = vec![
            FileEntry {
                rel: "main.js".into(),
                text: "function refill_action(user_id) { return user_id + 1; }".into(),
            },
            FileEntry {
                rel: "main.ts".into(),
                text: "function refill_action(user_id: number): number { return user_id + 1; }"
                    .into(),
            },
            FileEntry {
                rel: "Main.java".into(),
                text: "class Main { int refill_action(int user_id) { return user_id + 1; } }"
                    .into(),
            },
            FileEntry {
                rel: "main.go".into(),
                text: "func refill_action(user_id int) int { return user_id + 1 }".into(),
            },
        ];

        let out = transform_files(&files, &map).expect("transform");
        for (_, transformed) in out {
            assert!(transformed.contains("r1"));
            assert!(transformed.contains("u1"));
            assert!(!transformed.contains("refill_action"));
            assert!(!transformed.contains("user_id"));
        }
    }

    #[test]
    fn obfuscates_sql_inside_python_multiline_string() {
        let mut map = BTreeMap::new();
        map.insert("column1".into(), "x1".into());
        map.insert("table1".into(), "t1".into());
        map.insert("table2".into(), "t2".into());

        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "VAR_Q1 = \"\"\"\nSELECT u.column1\nFROM schema.table1 m\nJOIN schema.table2 u ON m.column1 = u.id\n\"\"\"\n".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("SELECT u.x1"));
        assert!(out[0].1.contains("FROM schema.t1 m"));
        assert!(out[0].1.contains("JOIN schema.t2 u"));
    }

    #[test]
    fn renames_object_identifier_before_member_access() {
        let mut map = BTreeMap::new();
        map.insert("profile".into(), "p1".into());

        let f = vec![FileEntry {
            rel: "main.py".into(),
            text: "profile = User()
print(profile.name)
"
            .into(),
        }];

        let out = transform_files(&f, &map).expect("transform");
        assert!(out[0].1.contains("p1 = User()"));
        assert!(out[0].1.contains("print(p1.name)"));
    }

    #[test]
    fn renames_identifiers_on_new_line_after_colon() {
        let mut map = BTreeMap::new();
        map.insert("source_rows".into(), "s1".into());

        let f = vec![FileEntry {
            rel: "main.py".into(),
            text: "def f():
    source_rows = [1]
    for x in source_rows:
        print(x)
"
            .into(),
        }];

        let out = transform_files(&f, &map).expect("transform");
        assert!(out[0].1.contains("s1 = [1]"));
        assert!(out[0].1.contains("in s1:"));
    }
    #[test]
    fn replaces_non_sql_strings_when_mapping_contains_identifier() {
        let mut map = BTreeMap::new();
        map.insert("table1".into(), "t1".into());

        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "message = \"table1 should stay in plain text\"\n".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("t1 should stay in plain text"));
    }

    #[test]
    fn keeps_language_keywords_even_if_present_in_mapping() {
        let map = BTreeMap::from([
            ("class".to_string(), "BrokenClassKw".to_string()),
            ("select".to_string(), "BrokenSelectKw".to_string()),
            ("new".to_string(), "BrokenNewKw".to_string()),
        ]);

        let files = vec![
            FileEntry {
                rel: "main.py".into(),
                text: "@dataclass\nclass User:\n    pass\n".into(),
            },
            FileEntry {
                rel: "main.sql".into(),
                text: "select user_id from users;\n".into(),
            },
            FileEntry {
                rel: "main.js".into(),
                text: "const m = new Map();\n".into(),
            },
        ];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("class User:"));
        assert!(out[1].1.contains("select user_id from users;"));
        assert!(out[2].1.contains("new Map()"));
    }

    #[test]
    fn obfuscates_python_named_argument_labels_when_present_in_mapping() {
        let map = BTreeMap::from([
            ("user_name".to_string(), "py_var_A1".to_string()),
            ("greet".to_string(), "py_method_A1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "def greet(user_name):\n    return user_name\n\ngreet(user_name='x')\n".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("def py_method_A1(py_var_A1):"));
        assert!(out[0].1.contains("return py_var_A1"));
        assert!(out[0].1.contains("py_method_A1(py_var_A1='x')"));
    }

    #[test]
    fn obfuscates_identifiers_inside_python_fstring_expressions() {
        let map = BTreeMap::from([
            ("cards".to_string(), "py_var_A1".to_string()),
            ("entity".to_string(), "py_var_B1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "total = f\"{len(cards)}:{entity.id}\"\n".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("f\"{len(py_var_A1)}:{py_var_B1.id}\""));
    }

    #[test]
    fn keeps_python_member_access_identifiers_unmodified() {
        let map = BTreeMap::from([
            ("environ".to_string(), "py_var_A1".to_string()),
            ("get".to_string(), "py_method_A1".to_string()),
            ("DataFrame".to_string(), "py_method_B1".to_string()),
            ("partner_id".to_string(), "py_var_B2".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "import os\nimport pandas as pd\n\nvalue = os.environ.get('X')\ndf = pd.DataFrame()\n_ = df[\"partner_id\"]\n".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("os.environ.get('X')"));
        assert!(out[0].1.contains("pd.DataFrame()"));
        assert!(out[0].1.contains("[\"py_var_B2\"]"));
    }

    #[test]
    fn obfuscates_python_class_methods_used_via_self_and_cls() {
        let map = BTreeMap::from([
            ("Service".to_string(), "ClassA1".to_string()),
            ("make_value".to_string(), "method_a1".to_string()),
            ("build".to_string(), "method_b1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "class Service:\n    def make_value(self):\n        return 1\n\n    @classmethod\n    def build(cls):\n        return cls()\n\n    def run(self):\n        return self.make_value() + self.build().make_value()\n"
                .into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("class ClassA1:"));
        assert!(out[0].1.contains("def method_a1(self):"));
        assert!(out[0].1.contains("def method_b1(cls):"));
        assert!(out[0].1.contains("self.method_a1()"));
        assert!(out[0].1.contains("self.method_b1().method_a1()"));
    }

    #[test]
    fn obfuscates_local_python_class_imports_and_constructor_kwargs() {
        let map = BTreeMap::from([
            ("CategoryUser".to_string(), "py_class_A1".to_string()),
            ("project_user_id".to_string(), "py_field_U1".to_string()),
        ]);
        let files = vec![
            FileEntry {
                rel: "models.py".into(),
                text: "class CategoryUser:\n    project_user_id: str\n".into(),
            },
            FileEntry {
                rel: "main.py".into(),
                text:
                    "from models import CategoryUser\n\nobj = CategoryUser(project_user_id='1')\n"
                        .into(),
            },
        ];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("class py_class_A1:"));
        assert!(out[1].1.contains("from models import py_class_A1"));
        assert!(out[1].1.contains("obj = py_class_A1(py_field_U1='1')"));
    }

    #[test]
    fn obfuscates_python_method_locals_used_in_kwargs() {
        let map = BTreeMap::from([
            ("user_ids".to_string(), "py_var_V6".to_string()),
            ("params".to_string(), "py_var_P1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "def f(users):\n    user_ids = [u.id for u in users]\n    return call(params={\"user_ids\": user_ids})\n"
                .into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("py_var_V6 = [u.id for u in users]"));
        assert!(out[0].1.contains("py_var_P1={\"py_var_V6\": py_var_V6}"));
    }
}
