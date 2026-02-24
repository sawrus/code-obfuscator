use std::path::Path;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Java,
    CSharp,
    CCpp,
    Go,
    Rust,
    Sql,
    Bash,
    Unknown,
}

pub fn detect_language(path: &Path, text: &str) -> Language {
    if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
        match ext {
            "py" => return Language::Python,
            "js" | "mjs" | "cjs" => return Language::JavaScript,
            "ts" | "tsx" => return Language::TypeScript,
            "java" => return Language::Java,
            "cs" => return Language::CSharp,
            "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" | "hh" => return Language::CCpp,
            "go" => return Language::Go,
            "rs" => return Language::Rust,
            "sql" => return Language::Sql,
            "sh" | "bash" => return Language::Bash,
            _ => {}
        }
    }

    if text.starts_with("#!/bin/bash") || text.starts_with("#!/usr/bin/env bash") {
        return Language::Bash;
    }
    Language::Unknown
}

pub fn is_keyword(lang: Language, s: &str) -> bool {
    keywords(lang).contains(&s) || keywords(lang).contains(&s.to_ascii_lowercase().as_str())
}

pub fn is_valid_identifier_for(lang: Language, candidate: &str) -> bool {
    if candidate.is_empty()
        || is_keyword(lang, candidate)
        || is_protected_system_name(candidate)
        || is_protected_entrypoint_name(candidate)
    {
        return false;
    }

    match lang {
        Language::Python
        | Language::JavaScript
        | Language::TypeScript
        | Language::Java
        | Language::CSharp
        | Language::CCpp
        | Language::Go
        | Language::Rust
        | Language::Bash
        | Language::Unknown => ident_re().is_match(candidate),
        Language::Sql => sql_ident_re().is_match(candidate),
    }
}

pub fn is_protected_system_name(s: &str) -> bool {
    PROTECTED_SYSTEM_NAMES.contains(&s)
}

pub fn is_protected_entrypoint_name(s: &str) -> bool {
    PROTECTED_ENTRYPOINTS.contains(&s)
}

fn ident_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").expect("valid regex"))
}

fn sql_ident_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_\$]*$").expect("valid regex"))
}

const PROTECTED_SYSTEM_NAMES: &[&str] = &[
    "__init__",
    "__name__",
    "__main__",
    "__file__",
    "constructor",
    "prototype",
    "toString",
    "valueOf",
    "String",
    "Object",
    "Array",
    "Error",
    "Exception",
    "System",
    "Console",
    "Program",
];

const PROTECTED_ENTRYPOINTS: &[&str] = &[
    "main",
    "Main",
    "Program",
    "_start",
    "WinMain",
    "DllMain",
    "init",
    "Start",
    "App",
    "Application",
    "run",
];

fn keywords(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &[
            "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
            "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
            "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
            "return", "try", "while", "with", "yield",
        ],
        Language::JavaScript | Language::TypeScript => &[
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "export",
            "extends",
            "finally",
            "for",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "let",
            "new",
            "return",
            "super",
            "switch",
            "this",
            "throw",
            "try",
            "typeof",
            "var",
            "void",
            "while",
            "with",
            "yield",
            "enum",
            "interface",
            "implements",
            "type",
            "namespace",
            "declare",
            "public",
            "private",
            "protected",
            "readonly",
            "as",
            "from",
        ],
        Language::Java => &[
            "abstract",
            "assert",
            "boolean",
            "break",
            "byte",
            "case",
            "catch",
            "char",
            "class",
            "const",
            "continue",
            "default",
            "do",
            "double",
            "else",
            "enum",
            "extends",
            "final",
            "finally",
            "float",
            "for",
            "goto",
            "if",
            "implements",
            "import",
            "instanceof",
            "int",
            "interface",
            "long",
            "native",
            "new",
            "package",
            "private",
            "protected",
            "public",
            "return",
            "short",
            "static",
            "strictfp",
            "super",
            "switch",
            "synchronized",
            "this",
            "throw",
            "throws",
            "transient",
            "try",
            "void",
            "volatile",
            "while",
            "true",
            "false",
            "null",
        ],
        Language::CSharp => &[
            "abstract",
            "as",
            "base",
            "bool",
            "break",
            "byte",
            "case",
            "catch",
            "char",
            "checked",
            "class",
            "const",
            "continue",
            "decimal",
            "default",
            "delegate",
            "do",
            "double",
            "else",
            "enum",
            "event",
            "explicit",
            "extern",
            "false",
            "finally",
            "fixed",
            "float",
            "for",
            "foreach",
            "goto",
            "if",
            "implicit",
            "in",
            "int",
            "interface",
            "internal",
            "is",
            "lock",
            "long",
            "namespace",
            "new",
            "null",
            "object",
            "operator",
            "out",
            "override",
            "params",
            "private",
            "protected",
            "public",
            "readonly",
            "ref",
            "return",
            "sbyte",
            "sealed",
            "short",
            "sizeof",
            "stackalloc",
            "static",
            "string",
            "struct",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "uint",
            "ulong",
            "unchecked",
            "unsafe",
            "ushort",
            "using",
            "virtual",
            "void",
            "volatile",
            "while",
        ],
        Language::CCpp => &[
            "auto",
            "break",
            "case",
            "char",
            "const",
            "continue",
            "default",
            "do",
            "double",
            "else",
            "enum",
            "extern",
            "float",
            "for",
            "goto",
            "if",
            "inline",
            "int",
            "long",
            "register",
            "restrict",
            "return",
            "short",
            "signed",
            "sizeof",
            "static",
            "struct",
            "switch",
            "typedef",
            "union",
            "unsigned",
            "void",
            "volatile",
            "while",
            "class",
            "namespace",
            "template",
            "typename",
            "public",
            "private",
            "protected",
            "virtual",
            "constexpr",
            "using",
            "new",
            "delete",
            "nullptr",
            "true",
            "false",
        ],
        Language::Go => &[
            "break",
            "case",
            "chan",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "fallthrough",
            "for",
            "func",
            "go",
            "goto",
            "if",
            "import",
            "interface",
            "map",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "switch",
            "type",
            "var",
        ],
        Language::Rust => &[
            "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
            "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
            "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
            "unsafe", "use", "where", "while", "async", "await", "dyn",
        ],
        Language::Sql => &[
            "select",
            "from",
            "where",
            "insert",
            "into",
            "update",
            "delete",
            "join",
            "left",
            "right",
            "full",
            "outer",
            "inner",
            "on",
            "group",
            "by",
            "order",
            "having",
            "limit",
            "offset",
            "create",
            "table",
            "view",
            "index",
            "drop",
            "alter",
            "and",
            "or",
            "not",
            "null",
            "true",
            "false",
            "values",
            "set",
            "as",
            "distinct",
            "union",
            "all",
            "case",
            "when",
            "then",
            "end",
            "primary",
            "key",
            "foreign",
            "references",
            "constraint",
            "database",
            "schema",
            "if",
            "exists",
            "postgresql",
        ],
        Language::Bash => &[
            "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac",
            "function", "in", "select", "until", "time", "coproc", "break", "continue", "return",
            "readonly", "local", "declare", "typeset", "export", "unset", "true", "false",
        ],
        Language::Unknown => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_by_extension() {
        assert_eq!(detect_language(&PathBuf::from("a.rs"), ""), Language::Rust);
        assert_eq!(
            detect_language(&PathBuf::from("a.py"), ""),
            Language::Python
        );
        assert_eq!(detect_language(&PathBuf::from("a.sql"), ""), Language::Sql);
    }

    #[test]
    fn detects_bash_by_shebang() {
        let lang = detect_language(&PathBuf::from("script"), "#!/usr/bin/env bash\necho 1");
        assert_eq!(lang, Language::Bash);
    }

    #[test]
    fn validates_identifiers_and_rejects_protected_names() {
        assert!(is_valid_identifier_for(Language::Rust, "Falcon1000"));
        assert!(!is_valid_identifier_for(Language::Rust, "1Falcon"));
        assert!(!is_valid_identifier_for(Language::Java, "class"));
        assert!(!is_valid_identifier_for(Language::Python, "__init__"));
        assert!(!is_valid_identifier_for(Language::Go, "main"));
    }
}
