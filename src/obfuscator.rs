use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::AppResult;
use crate::fs_ops::FileEntry;
use crate::language::detect_language;
use crate::transform;

pub fn transform_files(
    files: &[FileEntry],
    map: &BTreeMap<String, String>,
) -> AppResult<Vec<(PathBuf, String)>> {
    Ok(files
        .iter()
        .map(|f| {
            let lang = detect_language(&f.rel, &f.text);
            (f.rel.clone(), transform::apply_mapping(&f.text, lang, map))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_identifiers() {
        let mut map = BTreeMap::new();
        map.insert("Freeze".into(), "Go".into());
        let f = vec![FileEntry {
            rel: "a.py".into(),
            text: "Freeze(1) # Freeze in comment\ntext = \"Freeze\"".into(),
        }];
        let out = transform_files(&f, &map).expect("transform");
        assert_eq!(out[0].1, "Go(1) # Freeze in comment\ntext = \"Freeze\"");
    }
}
