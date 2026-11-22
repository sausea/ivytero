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

/// Line index of every `# ` / `## ` heading outside code fences. Each one ends
/// the previous block; only `##` blocks carrying an `RSS Key` are entries.
fn entry_headings(lines: &[&str]) -> Vec<usize> {
    let mut in_fence = false;
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if is_fence(line) {
            in_fence = !in_fence;
        } else if !in_fence && (line.starts_with("## ") || line.starts_with("# ")) {
            out.push(i);
        }
    }
    out
}

fn block_key<'a>(block: &[&'a str]) -> Option<&'a str> {
    if !block.first()?.starts_with("## ") {
        return None;
    }
    block
        .iter()
        .find_map(|l| l.trim().strip_prefix(KEY_PREFIX.trim_end()).map(str::trim))
        .filter(|k| !k.is_empty())
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
        let Some(key) = block_key(block) else {
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

/// Insert an `### AI 分析` section into the entry with `key` and flip
/// `- AI 状态：` to `done`. Re-analysis replaces the previous section.
/// `None` when the entry is absent or the analysis is empty.
pub fn annotate_entry(doc: &str, key: &str, analysis: &str) -> Option<String> {
    let safe = sanitize_body(analysis);
    if safe.is_empty() {
        return None;
    }
    let lines: Vec<&str> = doc.lines().collect();
    let heads = entry_headings(&lines);
    for (n, &start) in heads.iter().enumerate() {
        let end = heads.get(n + 1).copied().unwrap_or(lines.len());
        let block = &lines[start..end];
        if block_key(block) != Some(key) {
            continue;
        }
        // Drop any previous AI 分析 section so re-analysis replaces it.
        let mut cleaned: Vec<&str> = Vec::with_capacity(block.len());
        let mut skip = false;
        for line in block {
            let t = line.trim_start();
            if t == "### AI 分析" {
                skip = true;
                continue;
            }
            if skip && (t.starts_with("### ") || t.starts_with("## ")) {
                skip = false;
            }
            if !skip {
                cleaned.push(line);
            }
        }
        let body_at = cleaned
            .iter()
            .position(|l| l.trim() == "### 正文")
            .unwrap_or(cleaned.len());
        let mut new_block: Vec<String> = Vec::with_capacity(cleaned.len() + safe.lines().count() + 4);
        for (i, line) in cleaned.iter().enumerate() {
            if i == body_at {
                new_block.push("### AI 分析".to_string());
                new_block.push(String::new());
                new_block.extend(safe.lines().map(str::to_string));
                new_block.push(String::new());
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("- AI 状态：") {
                let indent = &line[..line.len() - trimmed.len()];
                new_block.push(format!("{indent}- AI 状态：done"));
            } else {
                new_block.push((*line).to_string());
            }
        }
        let mut out = lines[..start].join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&new_block.join("\n"));
        out.push('\n');
        out.push_str(&lines[end..].join("\n"));
        if doc.ends_with('\n') && !out.ends_with('\n') {
            out.push('\n');
        }
        return Some(out);
    }
    None
}

/// Drop the entry with `key` (and its leading `---` separator). `None` when absent.
pub fn remove_entry(doc: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = doc.lines().collect();
    let heads = entry_headings(&lines);
    for (n, &start) in heads.iter().enumerate() {
        let end = heads.get(n + 1).copied().unwrap_or(lines.len());
        if block_key(&lines[start..end]) != Some(key) {
            continue;
        }
        let trailing_rule = lines[start..end]
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|l| l.trim() == "---");
        let prev_is_entry = n > 0 && block_key(&lines[heads[n - 1]..start]).is_some();
        let mut cut_start = start;
        while cut_start > 0 && lines[cut_start - 1].trim().is_empty() {
            cut_start -= 1;
        }
        // Drop one separator: our own trailing `---`, else the one before us.
        if !trailing_rule && prev_is_entry && lines[cut_start - 1].trim() == "---" {
            cut_start -= 1;
        }
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
    fn removal_stops_at_user_headings_and_keeps_separators() {
        let mut doc = new_document("2026-11-21");
        for k in ["s/a1", "s/a2", "s/a3"] {
            doc = append_entry(&doc, &entry(k, k, "text"));
        }
        doc.push_str("\n# My journal\n\npersonal\n\n## Notes\n\nmore\n");
        let next = remove_entry(&doc, "s/a3").unwrap();
        assert!(next.contains("# My journal\n\npersonal"));
        assert!(next.contains("## Notes\n\nmore"));
        let mid = remove_entry(&doc, "s/a2").unwrap();
        assert_eq!(mid.matches("\n---\n").count(), 2, "{mid}");
        assert!(mid.contains("# My journal"));
        let first = remove_entry(&doc, "s/a1").unwrap();
        assert!(first.starts_with("---\ntype: rss-daily"));
        assert_eq!(find_entries(&first).len(), 2);
        assert!(remove_entry(&doc, "s/zz").is_none());
    }

    #[test]
    fn annotates_entry_and_replaces_previous_analysis() {
        let mut doc = new_document("2026-11-21");
        doc = append_entry(&doc, &entry("s/a1", "T", "body text"));
        doc = append_entry(&doc, &entry("s/a2", "U", "other"));
        let once = annotate_entry(&doc, "s/a1", "## 小结\n第一段。\n## 要点\n- 甲\n- 乙").expect("annotate");
        assert!(once.contains("### AI 分析"));
        assert!(once.contains("第一段。"));
        assert!(once.contains("- AI 状态：done"));
        assert!(!once.contains("\n## 小结"), "analysis headings must be shifted: {once}");
        assert!(once.contains("##### 小结"));
        assert!(once.contains("##### 要点"));
        assert_eq!(find_entries(&once).len(), 2, "{once}");
        assert!(find_entries(&once)[0].title == "T");
        let twice = annotate_entry(&once, "s/a1", "新的分析").expect("re-annotate");
        assert_eq!(twice.matches("### AI 分析").count(), 1, "{twice}");
        assert!(twice.contains("新的分析"));
        assert!(!twice.contains("第一段。"));
        // The second entry is untouched.
        assert!(twice.contains("- AI 状态：not_started"));
        assert!(annotate_entry(&twice, "s/zz", "x").is_none());
        assert!(annotate_entry(&twice, "s/a1", "  ").is_none());
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
