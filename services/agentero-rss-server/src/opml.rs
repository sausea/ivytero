//! OPML 1.0/2.0 subscription lists: import every `<outline xmlUrl=…>` with
//! its enclosing folder path, and export feeds nested by folder.

use crate::storage::FeedRow;
use std::collections::BTreeMap;

pub const MAX_OPML_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_OUTLINES: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpmlFeed {
    pub url: String,
    pub title: Option<String>,
    /// Enclosing folder outlines joined by `/` (a `/` inside a name becomes `／`).
    pub folder: String,
}

pub fn parse(text: &str) -> Result<Vec<OpmlFeed>, String> {
    if text.len() > MAX_OPML_BYTES {
        return Err("OPML file is too large".into());
    }
    // roxmltree rejects DTDs by default, so entity expansion is not possible.
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("invalid OPML: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name("opml") {
        return Err("invalid OPML: root element is not <opml>".into());
    }
    let mut out = Vec::new();
    for node in root.descendants().filter(|n| n.has_tag_name("outline")) {
        let Some(url) = node
            .attribute("xmlUrl")
            .or_else(|| node.attribute("xmlurl"))
            .map(str::trim)
            .filter(|u| !u.is_empty())
        else {
            continue;
        };
        if out.len() >= MAX_OUTLINES {
            return Err(format!("OPML lists more than {MAX_OUTLINES} feeds"));
        }
        let title = node
            .attribute("title")
            .or_else(|| node.attribute("text"))
            .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|t| !t.is_empty());
        let mut folders: Vec<String> = node
            .ancestors()
            .skip(1)
            .filter(|a| a.has_tag_name("outline") && a.attribute("xmlUrl").is_none())
            .filter_map(|a| a.attribute("title").or_else(|| a.attribute("text")))
            .map(|n| {
                n.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .replace('/', "／")
            })
            .filter(|n| !n.is_empty())
            .collect();
        folders.reverse();
        out.push(OpmlFeed {
            url: url.to_string(),
            title,
            folder: folders.join("/"),
        });
    }
    Ok(out)
}

fn escape(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' => s.push(' '),
            c => s.push(c),
        }
    }
    s
}

pub fn export(feeds: &[FeedRow], created: &str) -> String {
    let mut s =
        String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n");
    s.push_str(&format!(
        "  <head>\n    <title>Agentero cloud RSS</title>\n    <dateCreated>{}</dateCreated>\n  </head>\n  <body>\n",
        escape(created)
    ));
    let mut root = Folder::default();
    for f in feeds {
        let mut node = &mut root;
        for seg in f.folder.split('/').filter(|s| !s.is_empty()) {
            node = node.children.entry(seg.to_string()).or_default();
        }
        node.feeds.push(f);
    }
    root.write(&mut s, 2);
    s.push_str("  </body>\n</opml>\n");
    s
}

#[derive(Default)]
struct Folder<'a> {
    feeds: Vec<&'a FeedRow>,
    children: BTreeMap<String, Folder<'a>>,
}

impl Folder<'_> {
    fn write(&self, s: &mut String, depth: usize) {
        let pad = "  ".repeat(depth);
        for f in &self.feeds {
            let title = escape(&f.title);
            s.push_str(&format!(
                "{pad}<outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{}\"/>\n",
                escape(&f.url)
            ));
        }
        for (name, child) in &self.children {
            let name = escape(name);
            s.push_str(&format!(
                "{pad}<outline text=\"{name}\" title=\"{name}\">\n"
            ));
            child.write(s, depth + 1);
            s.push_str(&format!("{pad}</outline>\n"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_folder_paths_and_skips_folder_rows() {
        let text = r#"<?xml version="1.0"?><opml version="1.0"><head/><body>
            <outline text="Tech"><outline text=" AI / ML "><outline text="A &amp; B" xmlUrl="https://a.example/feed"/></outline></outline>
            <outline title="中文" text="ignored" xmlUrl=" https://b.example/rss "/>
            <outline text="no url"/></body></opml>"#;
        let feeds = parse(text).unwrap();
        assert_eq!(
            feeds,
            [
                OpmlFeed {
                    url: "https://a.example/feed".into(),
                    title: Some("A & B".into()),
                    folder: "Tech/AI ／ ML".into(),
                },
                OpmlFeed {
                    url: "https://b.example/rss".into(),
                    title: Some("中文".into()),
                    folder: String::new(),
                },
            ]
        );
    }

    #[test]
    fn rejects_dtd_and_non_opml() {
        let bomb = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaa">]><opml><body><outline xmlUrl="&a;"/></body></opml>"#;
        assert!(parse(bomb).is_err());
        assert!(parse("<rss/>").is_err());
        assert!(parse("not xml").is_err());
    }
}
