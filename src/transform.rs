use std::collections::{BTreeMap, BTreeSet};

use crate::language::{Language, is_keyword};

#[derive(Debug, Clone)]
enum TokenKind {
    Identifier,
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    text: String,
}

pub fn collect_terms(text: &str, lang: Language, out: &mut BTreeSet<String>) {
    let tokens = tokenize(text, lang);
    for i in 0..tokens.len() {
        if let TokenKind::Identifier = tokens[i].kind {
            let name = tokens[i].text.as_str();
            if name.len() >= 3 && should_rename_identifier(&tokens, i, lang) {
                out.insert(name.to_string());
            }
        }
    }
}

pub fn apply_mapping(text: &str, lang: Language, map: &BTreeMap<String, String>) -> String {
    let tokens = tokenize(text, lang);
    let mut out = String::with_capacity(text.len());
    for i in 0..tokens.len() {
        match tokens[i].kind {
            TokenKind::Identifier if should_rename_identifier(&tokens, i, lang) => {
                let name = tokens[i].text.as_str();
                if let Some(repl) = map.get(name) {
                    out.push_str(repl);
                } else {
                    out.push_str(name);
                }
            }
            _ => out.push_str(&tokens[i].text),
        }
    }
    out
}

fn should_rename_identifier(tokens: &[Token], idx: usize, lang: Language) -> bool {
    let name = tokens[idx].text.as_str();
    if is_keyword(lang, name) || is_builtin(lang, name) {
        return false;
    }
    if lang == Language::Python && is_python_dunder(name) {
        return false;
    }
    if in_import_path(tokens, idx, lang) || is_attribute_key(tokens, idx, lang) {
        return false;
    }
    true
}

fn is_python_dunder(s: &str) -> bool {
    s.starts_with("__") && s.ends_with("__")
}

fn is_builtin(lang: Language, s: &str) -> bool {
    let list: &[&str] = match lang {
        Language::Python => &[
            "print",
            "len",
            "str",
            "int",
            "list",
            "dict",
            "set",
            "tuple",
            "Exception",
            "object",
            "__name__",
            "__main__",
        ],
        Language::JavaScript | Language::TypeScript => &[
            "console", "window", "document", "JSON", "Math", "Promise", "Array", "Object",
        ],
        Language::Java => &["System", "String", "Object", "Integer", "Long", "Boolean"],
        Language::CSharp => &["Console", "String", "Int32", "Task", "Math"],
        Language::CCpp => &["std", "string", "vector", "cout", "cin", "printf", "size_t"],
        Language::Go => &["fmt", "string", "error", "int", "bool", "rune", "byte"],
        Language::Rust => &["std", "String", "Vec", "Option", "Result", "Self", "self"],
        Language::Sql => &["COUNT", "SUM", "MIN", "MAX", "AVG"],
        Language::Bash => &["PATH", "HOME", "PWD", "IFS", "BASH_SOURCE"],
        Language::Unknown => &[],
    };
    list.iter().any(|x| x.eq_ignore_ascii_case(s))
}

fn is_attribute_key(tokens: &[Token], idx: usize, lang: Language) -> bool {
    if !matches!(
        lang,
        Language::JavaScript | Language::TypeScript | Language::Python | Language::Sql
    ) {
        return false;
    }
    let next = next_significant(tokens, idx + 1);
    let prev = prev_significant(tokens, idx);
    match (prev, next) {
        (Some(p), Some(n))
            if tokens[n].text == ":" && (tokens[p].text == "{" || tokens[p].text == ",") =>
        {
            true
        }
        _ => false,
    }
}

fn in_import_path(tokens: &[Token], idx: usize, lang: Language) -> bool {
    let mut i = idx;
    while let Some(prev) = prev_significant(tokens, i) {
        let t = tokens[prev].text.as_str();
        if is_import_start(lang, t) {
            return true;
        }
        if matches!(t, ";" | "\n" | "{" | "}" | "(") {
            break;
        }
        i = prev;
    }

    if matches!(
        lang,
        Language::JavaScript | Language::TypeScript | Language::Python | Language::Rust
    ) {
        if let Some(prev) = prev_significant(tokens, idx) {
            if tokens[prev].text == "." {
                let mut j = prev;
                while let Some(pp) = prev_significant(tokens, j) {
                    if is_import_start(lang, tokens[pp].text.as_str()) {
                        return true;
                    }
                    if matches!(tokens[pp].text.as_str(), ";" | "\n" | "{" | "}" | "(") {
                        break;
                    }
                    j = pp;
                }
            }
        }
    }
    false
}

fn is_import_start(lang: Language, t: &str) -> bool {
    match lang {
        Language::Python => matches!(t, "import" | "from"),
        Language::JavaScript | Language::TypeScript => {
            matches!(t, "import" | "from" | "require" | "export")
        }
        Language::Java => matches!(t, "import" | "package"),
        Language::CSharp => matches!(t, "using" | "namespace"),
        Language::CCpp => t == "include",
        Language::Go => matches!(t, "import" | "package"),
        Language::Rust => matches!(t, "use" | "mod" | "crate"),
        Language::Sql => matches!(t, "from" | "join" | "into" | "update" | "table"),
        Language::Bash => matches!(t, "source"),
        Language::Unknown => false,
    }
}

fn prev_significant(tokens: &[Token], mut idx: usize) -> Option<usize> {
    while idx > 0 {
        idx -= 1;
        if !tokens[idx].text.trim().is_empty() {
            return Some(idx);
        }
    }
    None
}

fn next_significant(tokens: &[Token], mut idx: usize) -> Option<usize> {
    while idx < tokens.len() {
        if !tokens[idx].text.trim().is_empty() {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

fn tokenize(text: &str, lang: Language) -> Vec<Token> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_ident_start(c, lang) {
            let start = i;
            i += 1;
            while i < chars.len() && is_ident_continue(chars[i], lang) {
                i += 1;
            }
            out.push(Token {
                kind: TokenKind::Identifier,
                text: chars[start..i].iter().collect(),
            });
            continue;
        }

        if c == '\''
            || c == '"'
            || (c == '`' && matches!(lang, Language::JavaScript | Language::TypeScript))
        {
            let q = c;
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Token {
                kind: TokenKind::Other,
                text: chars[start..i.min(chars.len())].iter().collect(),
            });
            continue;
        }

        if c == '#' && matches!(lang, Language::Python | Language::Bash) {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push(Token {
                kind: TokenKind::Other,
                text: chars[start..i].iter().collect(),
            });
            continue;
        }
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' && lang == Language::Sql {
            let start = i;
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push(Token {
                kind: TokenKind::Other,
                text: chars[start..i].iter().collect(),
            });
            continue;
        }
        if c == '/'
            && i + 1 < chars.len()
            && chars[i + 1] == '/'
            && !matches!(lang, Language::Python | Language::Bash | Language::Sql)
        {
            let start = i;
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push(Token {
                kind: TokenKind::Other,
                text: chars[start..i].iter().collect(),
            });
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            out.push(Token {
                kind: TokenKind::Other,
                text: chars[start..i].iter().collect(),
            });
            continue;
        }

        out.push(Token {
            kind: TokenKind::Other,
            text: c.to_string(),
        });
        i += 1;
    }
    out
}

fn is_ident_start(c: char, lang: Language) -> bool {
    if lang == Language::Sql {
        c.is_ascii_alphabetic() || c == '_'
    } else {
        c.is_ascii_alphabetic() || c == '_' || (lang == Language::Bash && c == '$')
    }
}

fn is_ident_continue(c: char, lang: Language) -> bool {
    if lang == Language::Bash {
        c.is_ascii_alphanumeric() || c == '_' || c == '$'
    } else {
        c.is_ascii_alphanumeric() || c == '_'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_python_dunder() {
        let mut out = BTreeSet::new();
        collect_terms(
            "if __name__ == \"__main__\":\n    run_app()",
            Language::Python,
            &mut out,
        );
        assert!(!out.contains("__name__"));
        assert!(!out.contains("__main__"));
        assert!(out.contains("run_app"));
    }
}
