//! `notes/rss/YYYY-MM-DD.md`: one file per vault and local day, one `##` entry
//! per bookmarked article, located by its `> RSS Key: serverId/articleId` line.

pub const RSS_DIR: &str = "notes/rss";
const KEY_PREFIX: &str = "> RSS Key: ";

pub struct EntryFields<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub feed_title: &'a str,
    pub url: Option<&'a str>,
    pub published_at: Option<&'a str>,
    pub bookmarked_at: &'a str,
    pub tags: &'a [String],
    pub content_status: &'a str,
    pub body: &'a str,
}

/// An entry found in a daily file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundEntry {
    pub key: String,
    pub title: String,
    pub tags: Vec<String>,
    pub bookmarked_at: String,
    pub url: Option<String>,
    pub feed_title: String,
}

pub fn note_rel(archive_date: &str) -> String {
    format!("{RSS_DIR}/{archive_date}.md")
}

pub fn new_document(archive_date: &str) -> String {
    format!(
        "---\ntype: rss-daily\ndate: \"{archive_date}\"\ngenerated_by: agentero\n---\n\n# {archive_date} RSS 收藏\n"
    )
}

fn one_line(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Shift body headings below the entry's `###` sections, keep fences balanced
/// and neutralize lines that would read as another entry's key.
pub fn sanitize_body(body: &str) -> String {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        if is_fence(line) {
            in_fence = !in_fence;
            out.push(line.to_string());
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            continue;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        if level > 0 && line[level..].starts_with(' ') {
            let shifted = (level + 3).min(6);
            out.push(format!("{}{}", "#".repeat(shifted), &line[level..]));
        } else if line.trim_start().starts_with(KEY_PREFIX.trim_end()) {
            out.push(format!("\\{}", line.trim_start()));
        } else {
            out.push(line.to_string());
        }
    }
    if in_fence {
        out.push("```".to_string());
    }
    out.join("\n").trim().to_string()
}

pub fn render_entry(f: &EntryFields<'_>) -> String {
    let title = one_line(f.title);
    let title = if title.is_empty() {
        f.key.to_string()
    } else {
        title
    };
    let mut s = format!("## {title}\n\n{KEY_PREFIX}{}\n\n", f.key);
    s.push_str(&format!("- 来源：{}\n", one_line(f.feed_title)));
    if let Some(url) = f.url.filter(|u| !u.trim().is_empty()) {
        s.push_str(&format!("- 原文：{}\n", url.trim()));
    }
    if let Some(p) = f.published_at.filter(|p| p.len() >= 10) {
        s.push_str(&format!("- 发布时间：{}\n", &p[..10]));
    }
    s.push_str(&format!("- 收藏时间：{}\n", f.bookmarked_at));
    s.push_str(&format!("- 人工标签：{}\n", f.tags.join("、")));
    s.push_str(&format!("- 正文状态：{}\n", f.content_status));
    s.push_str("- AI 状态：not_started\n\n### 正文\n\n");
    let body = sanitize_body(f.body);
    if body.is_empty() {
        s.push_str("（未获取到正文，可打开原文查看）\n");
    } else {
        s.push_str(&body);
        s.push('\n');
    }
    s
}

/// Line index of every top-level `## ` heading outside code fences.
fn entry_headings(lines: &[&str]) -> Vec<usize> {
    let mut in_fence = false;
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if is_fence(line) {
            in_fence = !in_fence;
        } else if !in_fence && line.starts_with("## ") {
            out.push(i);
        }
    }
    out
}

fn meta(lines: &[&str], label: &str) -> Option<String> {
    let prefix = format!("- {label}：");
    lines.iter().find_map(|l| {
        l.trim()
            .strip_prefix(prefix.as_str())
            .map(|v| v.trim().to_string())
    })
}

pub fn find_entries(doc: &str) -> Vec<FoundEntry> {
    let lines: Vec<&str> = doc.lines().collect();
    let heads = entry_headings(&lines);
    let mut out = Vec::new();
    for (n, &start) in heads.iter().enumerate() {
        let end = heads.get(n + 1).copied().unwrap_or(lines.len());
        let block = &lines[start..end];
        let Some(key) = block
            .iter()
            .find_map(|l| l.trim().strip_prefix(KEY_PREFIX.trim_end()).map(str::trim))
            .filter(|k| !k.is_empty())
        else {
            continue;
        };
        let tags = meta(block, "人工标签")
            .map(|t| {
                t.split('、')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        out.push(FoundEntry {
            key: key.to_string(),
            title: block[0][3..].trim().to_string(),
            tags,
            bookmarked_at: meta(block, "收藏时间").unwrap_or_default(),
            url: meta(block, "原文"),
            feed_title: meta(block, "来源").unwrap_or_default(),
        });
    }
    out
}

pub fn contains_key(doc: &str, key: &str) -> bool {
    find_entries(doc).iter().any(|e| e.key == key)
}

pub fn append_entry(doc: &str, entry: &str) -> String {
    let mut out = doc.trim_end().to_string();
    out.push_str("\n\n");
    if !find_entries(doc).is_empty() {
        out.push_str("---\n\n");
    }
    out.push_str(entry.trim_end());
    out.push('\n');
    out
}

/// Drop the entry with `key` (and its leading `---` separator). `None` when absent.
pub fn remove_entry(doc: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = doc.lines().collect();
    let heads = entry_headings(&lines);
    for (n, &start) in heads.iter().enumerate() {
        let end = heads.get(n + 1).copied().unwrap_or(lines.len());
        let has_key = lines[start..end]
            .iter()
            .any(|l| l.trim().strip_prefix(KEY_PREFIX.trim_end()).map(str::trim) == Some(key));
        if !has_key {
            continue;
        }
        let mut cut_start = start;
        while cut_start > 0 && lines[cut_start - 1].trim().is_empty() {
            cut_start -= 1;
        }
        if cut_start > 0 && lines[cut_start - 1].trim() == "---" && n > 0 {
            cut_start -= 1;
        }
        // For the first entry, its trailing `---` lies inside [start, end).
        let cut_end = end;
        let mut kept: Vec<&str> = lines[..cut_start].to_vec();
        kept.extend_from_slice(&lines[cut_end..]);
        let mut out = kept.join("\n").trim_end().to_string();
        out.push('\n');
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, title: &str, body: &str) -> String {
        render_entry(&EntryFields {
            key,
            title,
            feed_title: "Feed",
            url: Some("https://example.org/a"),
            published_at: Some("2026-11-15T10:00:00.000Z"),
            bookmarked_at: "2026-11-21 09:35",
            tags: &["GPR".into(), "信号处理".into()],
            content_status: "partial",
            body,
        })
    }

    #[test]
    fn merges_same_day_entries_and_finds_keys() {
        let mut doc = new_document("2026-11-21");
        for (k, t) in [("s/a1", "Same"), ("s/a2", "Same"), ("s/a3", "Other")] {
            doc = append_entry(&doc, &entry(k, t, "# Heading\nbody\n> RSS Key: s/forged"));
        }
        let found = find_entries(&doc);
        assert_eq!(
            found.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            ["s/a1", "s/a2", "s/a3"]
        );
        assert_eq!(found[0].tags, ["GPR", "信号处理"]);
        assert!(doc.contains("#### Heading"));
        assert!(!contains_key(&doc, "s/forged"));
        assert!(doc.contains("- 发布时间：2026-11-15"));
    }

    #[test]
    fn removes_only_the_target_entry_and_keeps_user_notes() {
        let mut doc = new_document("2026-11-21");
        for k in ["s/a1", "s/a2", "s/a3"] {
            doc = append_entry(&doc, &entry(k, k, "text"));
        }
        let other = remove_entry(&doc, "s/a2").expect("present");
        assert_eq!(
            find_entries(&other)
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>(),
            ["s/a1", "s/a3"]
        );
        for k in ["s/a1", "s/a2", "s/a3"] {
            doc = remove_entry(&doc, k).expect("present");
            assert!(!contains_key(&doc, k));
            assert!(!doc.contains("---\n\n---"));
        }
        assert!(find_entries(&doc).is_empty());
        assert!(doc.starts_with("---\ntype: rss-daily"));
        assert!(remove_entry(&doc, "s/a1").is_none());
    }

    #[test]
    fn closes_unbalanced_fences() {
        let doc = append_entry(
            &new_document("2026-11-21"),
            &entry("s/a1", "T", "```\n## not a heading"),
        );
        let doc = append_entry(&doc, &entry("s/a2", "U", "x"));
        assert_eq!(find_entries(&doc).len(), 2);
    }
}
