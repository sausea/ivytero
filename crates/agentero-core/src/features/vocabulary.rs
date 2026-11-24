//! Vocabulary notebook (生词本): dictionary entries already shown by the
//! selection-translate card, stored as one Markdown note in the Vault.
//! The note is plain Markdown so it syncs with the Vault like any note.

use crate::error::AppError;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Vault-relative location of the notebook.
const NOTE_REL: &str = "notes/生词本.md";
const NOTE_HEADER: &str = "# 生词本\n";
const MAX_WORD_CHARS: usize = 120;

fn bad(code: &str) -> AppError {
    AppError::message(format!("vocabulary.{code}"))
}

fn vault_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Default::default)
}

/// Strip code fences and headings so the entry cannot break the note's
/// `## word` section structure.
fn sanitize_entry(raw: &str) -> String {
    raw.lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                trimmed.trim_start_matches('#').trim_start().to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn is_heading(line: &str, level: usize) -> bool {
    let t = line.trim_start();
    t.starts_with(&"#".repeat(level)) && {
        let rest = t.trim_start_matches('#');
        rest.starts_with(' ') || rest.is_empty()
    }
}

fn section_key(line: &str) -> Option<&str> {
    let t = line.trim();
    let rest = t.strip_prefix("## ")?;
    (!rest.trim().is_empty()).then_some(rest.trim())
}

/// Insert or replace the `## {word}` section. Returns the new document, or
/// `None` when the word is empty or contains heading/newline characters.
pub fn upsert_entry(doc: Option<&str>, word: &str, entry: &str) -> Option<String> {
    let word = word.trim();
    if word.is_empty()
        || word.chars().any(|c| c == '\n' || c == '\r' || c == '#')
        || word.chars().count() > MAX_WORD_CHARS
    {
        return None;
    }
    let mut lines: Vec<&str> = doc.unwrap_or("").lines().collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(NOTE_HEADER.trim_end());
        lines.push("");
    }
    let block: Vec<String> = format!("## {word}\n\n{entry}")
        .lines()
        .map(str::to_string)
        .collect();
    let word_lower = word.to_lowercase();
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(key) = section_key(line) {
            if key.to_lowercase() == word_lower {
                start = Some(i);
                break;
            }
        }
    }
    if let Some(start) = start {
        let mut end = lines.len();
        for (i, line) in lines.iter().enumerate().skip(start + 1) {
            if is_heading(line, 2) {
                end = i;
                break;
            }
        }
        let mut next: Vec<String> = lines[..start].iter().map(|s| s.to_string()).collect();
        while next.last().is_some_and(|l| l.trim().is_empty()) {
            next.pop();
        }
        next.push(String::new());
        next.extend(block);
        if end < lines.len() {
            next.push(String::new());
            next.extend(lines[end..].iter().map(|s| s.to_string()));
        }
        let mut out = next.join("\n");
        out.push('\n');
        Some(out)
    } else {
        let mut next: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        while next.last().is_some_and(|l| l.trim().is_empty()) {
            next.pop();
        }
        next.push(String::new());
        next.extend(block);
        let mut out = next.join("\n");
        out.push('\n');
        Some(out)
    }
}

/// Upsert the dictionary `entry` the reader already saw for `word` into the
/// Vault notebook. Returns the entry Markdown that was written.
pub fn add_word(vault_path: &str, word: &str, entry: &str) -> Result<String, AppError> {
    let word = word.trim();
    if word.is_empty()
        || word.chars().any(|c| c == '\n' || c == '\r' || c == '#')
        || word.chars().count() > MAX_WORD_CHARS
    {
        return Err(bad("invalid"));
    }
    let entry = sanitize_entry(entry);
    if entry.is_empty() {
        return Err(bad("empty"));
    }
    let vault = crate::fs::resolve_vault(vault_path).map_err(|_| bad("no_vault"))?;
    let path: PathBuf = vault.join(NOTE_REL);
    let _guard = vault_lock().lock().unwrap_or_else(|e| e.into_inner());
    let before = std::fs::read_to_string(&path).ok();
    let next = upsert_entry(before.as_deref(), word, &entry).ok_or_else(|| bad("invalid"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::fs::atomic_write(&path, next.as_bytes())?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_note_with_header() {
        let doc = upsert_entry(None, "apple", "/ˈæp.əl/\n**n.** 苹果").unwrap();
        assert!(doc.starts_with("# 生词本\n"));
        assert!(doc.contains("## apple\n\n/ˈæp.əl/"));
    }

    #[test]
    fn replaces_existing_entry_case_insensitively() {
        let first = upsert_entry(None, "Apple", "old").unwrap();
        let second = upsert_entry(Some(&first), "apple", "new").unwrap();
        assert_eq!(
            second.matches("## apple").count() + second.matches("## Apple").count(),
            1
        );
        assert!(second.contains("new"));
        assert!(!second.contains("old"));
    }

    #[test]
    fn keeps_other_sections() {
        let first = upsert_entry(None, "one", "entry one").unwrap();
        let second = upsert_entry(Some(&first), "two", "entry two").unwrap();
        assert!(second.contains("## one"));
        assert!(second.contains("## two"));
        let third = upsert_entry(Some(&second), "one", "entry one v2").unwrap();
        assert!(third.contains("entry one v2"));
        assert!(third.contains("## two"));
        assert!(!third.contains("entry one\n"));
    }

    #[test]
    fn rejects_invalid_words() {
        assert!(upsert_entry(None, "", "x").is_none());
        assert!(upsert_entry(None, "a\nb", "x").is_none());
        assert!(upsert_entry(None, "## hack", "x").is_none());
        assert!(upsert_entry(None, &"x".repeat(200), "x").is_none());
    }

    #[test]
    fn sanitize_strips_fences_and_headings() {
        let raw = "```md\n# Apple\n## sense\n**n.** 苹果\n```\ntrailing";
        let out = sanitize_entry(raw);
        assert!(!out.contains("```"));
        assert!(!out.contains("# Apple"));
        assert!(out.contains("**n.** 苹果"));
        assert!(out.contains("trailing"));
    }
}
