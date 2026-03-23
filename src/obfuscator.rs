use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::error::AppResult;
use crate::fs_ops::FileEntry;
use crate::language::{Language, detect_language, is_keyword};

pub fn transform_files_global(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    transform_files_global_with_progress(files, map, |_, _| {})
}

pub fn transform_files_global_with_progress<F>(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
    mut on_progress: F,
) -> AppResult<Vec<(PathBuf, String)>>
where
    F: FnMut(usize, usize),
{
    let total = files.len();
    let mut transformed = Vec::with_capacity(total);
    for (idx, file) in files.iter().enumerate() {
        transformed.push((file.rel.clone(), apply_global_mapping(&file.text, map)));
        on_progress(idx + 1, total);
    }
    Ok(transformed)
}

fn apply_global_mapping(text: &str, map: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (from, to) in map {
        out = replace_global_token(&out, from, to);
    }
    out
}

fn replace_global_token(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if rest.starts_with(from) && has_global_boundary(text, i, from.len()) {
            out.push_str(to);
            i += from.len();
            continue;
        }

        let ch = rest.chars().next().expect("char");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn has_global_boundary(text: &str, start: usize, len: usize) -> bool {
    let before_ok = text[..start]
        .chars()
        .next_back()
        .map(|c| !c.is_alphanumeric())
        .unwrap_or(true);
    let after_ok = text[start + len..]
        .chars()
        .next()
        .map(|c| !c.is_alphanumeric())
        .unwrap_or(true);
    before_ok && after_ok
}

pub fn transform_files(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    transform_files_with_progress(files, map, |_, _| {})
}

pub fn transform_files_with_progress<F>(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
    mut on_progress: F,
) -> AppResult<Vec<(PathBuf, String)>>
where
    F: FnMut(usize, usize),
{
    let globally_imported_python_symbols = collect_python_imported_symbols_from_files(files);
    let python_dataclass_index = collect_python_dataclass_index(files);
    let total = files.len();
    let mut transformed = Vec::with_capacity(total);
    for (idx, file) in files.iter().enumerate() {
        let lang = detect_language(&file.rel, &file.text);
        transformed.push((
            file.rel.clone(),
            apply_rules(
                &file.text,
                map,
                lang,
                &globally_imported_python_symbols,
                &python_dataclass_index,
            ),
        ));
        on_progress(idx + 1, total);
    }
    Ok(transformed)
}

fn apply_rules(
    text: &str,
    map: &BTreeMap<String, String>,
    lang: Language,
    globally_imported_python_symbols: &BTreeSet<String>,
    python_dataclass_index: &PythonDataclassIndex,
) -> String {
    let python_imports = if matches!(lang, Language::Python) {
        collect_python_imported_symbols(text)
    } else {
        BTreeSet::new()
    };
    let python_parameters = if matches!(lang, Language::Python) {
        collect_python_parameter_names(text)
    } else {
        BTreeSet::new()
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if let Some((next, segment, kind)) = scan_string_or_comment(text, i, lang) {
            if matches!(kind, SegmentKind::String) {
                out.push_str(&maybe_obfuscate_sql_in_string(
                    segment,
                    map,
                    lang,
                    python_dataclass_index,
                ));
            } else {
                out.push_str(segment);
            }
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
                || is_keyword(lang, token)
                || is_python_builtin_identifier(token, lang)
                || is_python_def_parameter(text, start, lang)
                || should_preserve_python_keyword_argument_label(
                    text,
                    start,
                    i,
                    token,
                    lang,
                    python_dataclass_index,
                )
                || python_imports.contains(token)
                || python_parameters.contains(token)
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
    python_dataclass_index: &PythonDataclassIndex,
) -> String {
    if matches!(lang, Language::Sql) {
        return segment.to_string();
    }

    let Some((prefix_len, quote_len, is_fstring)) = string_parts(segment, lang) else {
        return segment.to_string();
    };

    let content = &segment[prefix_len + quote_len..segment.len() - quote_len];
    let fstring_content = if is_fstring {
        obfuscate_python_fstring_expressions(content, map, python_dataclass_index)
    } else {
        content.to_string()
    };

    if !looks_like_sql(&fstring_content) {
        if is_fstring {
            return rebuild_string(segment, prefix_len, quote_len, &fstring_content);
        }
        return segment.to_string();
    }

    let obfuscated = apply_rules(
        &fstring_content,
        map,
        Language::Sql,
        &BTreeSet::new(),
        python_dataclass_index,
    );
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

fn obfuscate_python_fstring_expressions(
    content: &str,
    map: &BTreeMap<String, String>,
    python_dataclass_index: &PythonDataclassIndex,
) -> String {
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
                let expr_obf = apply_rules(
                    &expr,
                    map,
                    Language::Python,
                    &BTreeSet::new(),
                    python_dataclass_index,
                );
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

#[derive(Default)]
struct PythonDataclassIndex {
    classes: BTreeSet<String>,
    fields_by_class: BTreeMap<String, BTreeSet<String>>,
    bases_by_class: BTreeMap<String, Vec<String>>,
}

impl PythonDataclassIndex {
    fn class_has_field(&self, class_name: &str, field: &str) -> bool {
        let mut visited = BTreeSet::new();
        self.class_has_field_recursive(class_name, field, &mut visited)
    }

    fn class_has_field_recursive(
        &self,
        class_name: &str,
        field: &str,
        visited: &mut BTreeSet<String>,
    ) -> bool {
        if !visited.insert(class_name.to_string()) {
            return false;
        }
        if self
            .fields_by_class
            .get(class_name)
            .is_some_and(|fields| fields.contains(field))
        {
            return true;
        }
        self.bases_by_class.get(class_name).is_some_and(|bases| {
            bases
                .iter()
                .any(|base| self.class_has_field_recursive(base, field, visited))
        })
    }
}

fn collect_python_dataclass_index(files: &[FileEntry]) -> PythonDataclassIndex {
    let mut out = PythonDataclassIndex::default();
    for file in files {
        if !matches!(detect_language(&file.rel, &file.text), Language::Python) {
            continue;
        }
        collect_python_dataclass_index_from_text(&file.text, &mut out);
    }
    out
}

fn collect_python_dataclass_index_from_text(text: &str, out: &mut PythonDataclassIndex) {
    let class_re = Regex::new(r"^([ \t]*)class\s+([A-Za-z_][A-Za-z0-9_]*)(?:\(([^)]*)\))?\s*:")
        .expect("regex");
    let field_re = Regex::new(r"^([A-Za-z_][A-Za-z0-9_]*)\s*:").expect("regex");
    let lines: Vec<&str> = text.lines().collect();

    let mut i = 0;
    let mut saw_dataclass_decorator = false;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if trimmed.starts_with('@') {
            if is_dataclass_decorator(trimmed) {
                saw_dataclass_decorator = true;
            }
            i += 1;
            continue;
        }

        if let Some(cap) = class_re.captures(line) {
            let class_indent = cap.get(1).map(|m| m.as_str().len()).unwrap_or_default();
            let class_name = cap
                .get(2)
                .map(|m| m.as_str())
                .unwrap_or_default()
                .to_string();

            if saw_dataclass_decorator {
                out.classes.insert(class_name.clone());
                let bases =
                    parse_python_base_classes(cap.get(3).map(|m| m.as_str()).unwrap_or_default());
                if !bases.is_empty() {
                    out.bases_by_class.insert(class_name.clone(), bases);
                }
                let (fields, next_i) =
                    collect_python_dataclass_fields(&lines, i + 1, class_indent, &field_re);
                out.fields_by_class
                    .entry(class_name)
                    .or_default()
                    .extend(fields);
                saw_dataclass_decorator = false;
                i = next_i;
                continue;
            }

            saw_dataclass_decorator = false;
            i += 1;
            continue;
        }

        if !trimmed.is_empty() {
            saw_dataclass_decorator = false;
        }
        i += 1;
    }
}

fn is_dataclass_decorator(trimmed_line: &str) -> bool {
    let Some(rest) = trimmed_line.strip_prefix('@') else {
        return false;
    };
    let decorator = rest
        .split(|c: char| c == '(' || c.is_whitespace())
        .next()
        .unwrap_or_default();
    matches!(decorator, "dataclass" | "dataclasses.dataclass")
}

fn parse_python_base_classes(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter_map(|part| {
            let mut candidate = part.trim();
            if candidate.is_empty() {
                return None;
            }
            if let Some(idx) = candidate.find('[') {
                candidate = &candidate[..idx];
            }
            if let Some(idx) = candidate.find('(') {
                candidate = &candidate[..idx];
            }
            let name = candidate.rsplit('.').next().unwrap_or_default().trim();
            if is_plain_identifier(name) {
                return Some(name.to_string());
            }
            None
        })
        .collect()
}

fn collect_python_dataclass_fields(
    lines: &[&str],
    start_idx: usize,
    class_indent: usize,
    field_re: &Regex,
) -> (BTreeSet<String>, usize) {
    let mut out = BTreeSet::new();
    let mut i = start_idx;
    let mut direct_body_indent = None;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        if indent <= class_indent {
            break;
        }

        if direct_body_indent.is_none() {
            direct_body_indent = Some(indent);
        }

        if direct_body_indent == Some(indent)
            && let Some(cap) = field_re.captures(line.trim_start())
        {
            let field = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            if !matches!(field, "self" | "cls") {
                out.insert(field.to_string());
            }
        }

        i += 1;
    }

    (out, i)
}

fn should_preserve_python_keyword_argument_label(
    text: &str,
    start: usize,
    end: usize,
    token: &str,
    lang: Language,
    python_dataclass_index: &PythonDataclassIndex,
) -> bool {
    if !is_python_keyword_argument_label(text, start, end, lang) {
        return false;
    }
    !is_python_dataclass_keyword_argument_label(text, start, token, lang, python_dataclass_index)
}

fn is_python_dataclass_keyword_argument_label(
    text: &str,
    start: usize,
    token: &str,
    lang: Language,
    python_dataclass_index: &PythonDataclassIndex,
) -> bool {
    if !matches!(lang, Language::Python) {
        return false;
    }
    let Some(call_name) = find_enclosing_call_name(text, start) else {
        return false;
    };
    python_dataclass_index.class_has_field(&call_name, token)
}

fn find_enclosing_call_name(text: &str, token_start: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = token_start;
    let mut depth = 0usize;

    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' => {
                if depth == 0 {
                    return find_call_name_before_paren(text, i);
                }
                depth -= 1;
            }
            b'[' | b'{' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }

    None
}

fn find_call_name_before_paren(text: &str, open_paren_idx: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut end = open_paren_idx;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }

    let mut start = end;
    while start > 0 {
        let b = bytes[start - 1];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' {
            start -= 1;
            continue;
        }
        break;
    }

    let expr = &text[start..end];
    let name = expr.rsplit('.').next().unwrap_or_default().trim();
    if is_plain_identifier(name) {
        return Some(name.to_string());
    }
    None
}

fn is_plain_identifier(token: &str) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_ident_start(first) {
        return false;
    }
    chars.all(is_ident_continue)
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

    let mut prev = None;
    for c in text[..start].chars().rev() {
        if c == '\n' || c == '\r' {
            break;
        }
        if c.is_whitespace() {
            continue;
        }
        prev = Some(c);
        break;
    }

    matches!(prev, Some('.' | ':' | '#'))
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
    fn deep_mode_preserves_python_import_path_but_global_mode_replaces_it() {
        let map = BTreeMap::from([("pkg".to_string(), "xpkg".to_string())]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "from pkg.mod import greet\nprint(pkg)\n".into(),
        }];

        let deep = transform_files(&files, &map).expect("deep transform");
        assert!(deep[0].1.contains("from pkg.mod import greet"));
        assert!(deep[0].1.contains("print(xpkg)"));

        let global = transform_files_global(&files, &map).expect("global transform");
        assert!(global[0].1.contains("from xpkg.mod import greet"));
        assert!(global[0].1.contains("print(xpkg)"));
    }

    #[test]
    fn global_mode_replaces_in_strings_and_comments() {
        let map = BTreeMap::from([("run_task".to_string(), "Launch".to_string())]);
        let f = vec![FileEntry {
            rel: "a.py".into(),
            text: "# run_task\nprint(\"run_task\")\nrun_task()".into(),
        }];
        let out = transform_files_global(&f, &map).expect("transform");
        assert_eq!(out[0].1, "# Launch\nprint(\"Launch\")\nLaunch()");
    }

    #[test]
    fn global_mode_replaces_inside_snake_case_but_not_larger_alnum_tokens() {
        let map = BTreeMap::from([("bs".to_string(), "mmm".to_string())]);
        let f = vec![FileEntry {
            rel: "a.sql".into(),
            text: "bs.users %(bs_user_ids)s xbs9".into(),
        }];

        let out = transform_files_global(&f, &map).expect("transform");
        assert_eq!(out[0].1, "mmm.users %(mmm_user_ids)s xbs9");
    }

    #[test]
    fn global_mode_does_not_replace_inside_larger_alnum_token() {
        let map = BTreeMap::from([("bs".to_string(), "mmm".to_string())]);
        let f = vec![FileEntry {
            rel: "a.txt".into(),
            text: "xbs9".into(),
        }];

        let out = transform_files_global(&f, &map).expect("transform");
        assert_eq!(out[0].1, "xbs9");
    }

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
    fn does_not_obfuscate_non_sql_strings() {
        let mut map = BTreeMap::new();
        map.insert("table1".into(), "t1".into());

        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "message = \"table1 should stay in plain text\"\n".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("table1 should stay in plain text"));
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
    fn keeps_python_named_argument_labels_and_params_unmodified() {
        let map = BTreeMap::from([
            ("user_name".to_string(), "py_var_A1".to_string()),
            ("greet".to_string(), "py_method_A1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "def greet(user_name):\n    return user_name\n\ngreet(user_name='x')\n".into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("def py_method_A1(user_name):"));
        assert!(out[0].1.contains("return user_name"));
        assert!(out[0].1.contains("py_method_A1(user_name='x')"));
    }

    #[test]
    fn obfuscates_dataclass_constructor_keyword_labels() {
        let map = BTreeMap::from([
            ("CategoryUser".to_string(), "PyClass_A1".to_string()),
            ("project".to_string(), "py_field_A1".to_string()),
            ("project_user_id".to_string(), "py_field_B1".to_string()),
            ("cards".to_string(), "py_field_C1".to_string()),
            ("code".to_string(), "py_field_D1".to_string()),
            ("category_id".to_string(), "py_field_E1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "from dataclasses import dataclass\n\n@dataclass\nclass User:\n    project: str = \"\"\n    project_user_id: str = \"\"\n    cards: str = \"\"\n    code: str = \"\"\n\n@dataclass\nclass CategoryUser(User):\n    category_id: int = 0\n\nCategoryUser(\n    project=self.project,\n    project_user_id=payload[\"user_id\"],\n    cards=payload[\"cards\"],\n    code=payload[\"code\"],\n    category_id=self.category_id,\n)\n"
                .into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("PyClass_A1("));
        assert!(out[0].1.contains("py_field_A1=self.project"));
        assert!(out[0].1.contains("py_field_B1=payload[\"user_id\"]"));
        assert!(out[0].1.contains("py_field_C1=payload[\"cards\"]"));
        assert!(out[0].1.contains("py_field_D1=payload[\"code\"]"));
        assert!(out[0].1.contains("py_field_E1=self.category_id"));
    }

    #[test]
    fn keeps_external_base_fields_in_dataclass_constructor_labels() {
        let map = BTreeMap::from([
            ("CategoryUser".to_string(), "PyClass_A1".to_string()),
            ("project".to_string(), "py_field_A1".to_string()),
            ("project_user_id".to_string(), "py_field_B1".to_string()),
            ("user_id".to_string(), "py_field_C1".to_string()),
            ("category_id".to_string(), "py_field_D1".to_string()),
        ]);
        let files = vec![FileEntry {
            rel: "main.py".into(),
            text: "from dataclasses import dataclass\nfrom apiutil.models import User\n\n@dataclass\nclass MidUser(User):\n    user_id: str = \"\"\n\n@dataclass\nclass CategoryUser(MidUser):\n    category_id: int = 0\n\nCategoryUser(\n    project=self.project,\n    project_user_id=payload[\"project_user_id\"],\n    user_id=payload[\"user_id\"],\n    category_id=self.category_id,\n)\n"
                .into(),
        }];

        let out = transform_files(&files, &map).expect("transform");
        assert!(out[0].1.contains("from apiutil.models import User"));
        assert!(out[0].1.contains("class MidUser(User):"));
        assert!(out[0].1.contains("class PyClass_A1(MidUser):"));
        assert!(out[0].1.contains("PyClass_A1("));
        assert!(out[0].1.contains("project=self.project"));
        assert!(
            out[0]
                .1
                .contains("project_user_id=payload[\"project_user_id\"]")
        );
        assert!(out[0].1.contains("py_field_C1=payload[\"user_id\"]"));
        assert!(out[0].1.contains("py_field_D1=self.category_id"));
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
        assert!(out[0].1.contains("[\"partner_id\"]"));
    }
}
