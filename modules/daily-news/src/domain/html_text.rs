//! Pure HTML-to-text extraction primitives.
//!
//! These helpers perform deterministic string decomposition with no I/O or
//! framework dependencies, so both domain/application logic and infrastructure
//! adapters can share them without violating the dependency rule.

pub fn clean_html_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn is_block_tag(t: &str) -> bool {
    matches!(
        t,
        "p" | "div" | "br" | "li" | "tr" | "td" | "article" | "section" | "blockquote"
    ) || (t.len() == 2 && t.starts_with('h') && t.ends_with(|c: char| ('1'..='6').contains(&c)))
}

fn starts_with_ci(chars: &[char], needle: &str) -> bool {
    let mut source = chars.iter();
    needle.chars().all(|expected| {
        source
            .next()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(&expected))
    })
}

fn starts_with_tag_ci(chars: &[char], needle: &str) -> bool {
    if !starts_with_ci(chars, needle) {
        return false;
    }
    match chars.get(needle.chars().count()) {
        None => true,
        Some(c) => c.is_whitespace() || *c == '>' || *c == '/',
    }
}

fn skip_comment(chars: &[char], mut index: usize) -> usize {
    index += 4;
    while index + 2 < chars.len() {
        if chars[index] == '-' && chars[index + 1] == '-' && chars[index + 2] == '>' {
            return index + 3;
        }
        index += 1;
    }
    chars.len()
}

pub fn extract_main_text(html: &str) -> String {
    let chars: Vec<char> = html.chars().collect();
    let mut cleaned = String::with_capacity(html.len());
    let mut tag_buf = String::with_capacity(32);
    let mut quote: Option<char> = None;
    let mut in_tag = false;
    let mut raw_text_tag: Option<&'static str> = None;
    let mut index = 0;

    while index < chars.len() {
        let c = chars[index];

        if let Some(raw_tag) = raw_text_tag {
            let closing = format!("</{raw_tag}");
            if c == '<' && starts_with_tag_ci(&chars[index..], &closing) {
                while index < chars.len() && chars[index] != '>' {
                    index += 1;
                }
                index = (index + 1).min(chars.len());
                raw_text_tag = None;
                cleaned.push(' ');
            } else {
                index += 1;
            }
            continue;
        }

        if in_tag {
            if let Some(active_quote) = quote {
                if c == active_quote {
                    quote = None;
                }
            } else if c == '"' || c == '\'' {
                quote = Some(c);
            } else if c == '>' {
                in_tag = false;
                let tag_lower = tag_buf.to_ascii_lowercase();
                let tag_name = tag_lower
                    .trim_start_matches('/')
                    .trim_end_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                let is_closing = tag_lower.starts_with('/');
                let is_self_closing = tag_lower.trim_end().ends_with('/');
                if !is_closing && !is_self_closing && (tag_name == "script" || tag_name == "style") {
                    raw_text_tag = Some(if tag_name == "script" { "script" } else { "style" });
                } else if is_block_tag(tag_name) {
                    cleaned.push(' ');
                }
                tag_buf.clear();
            } else {
                tag_buf.push(c);
            }
            index += 1;
            continue;
        }

        if c == '<' {
            if starts_with_ci(&chars[index..], "<!--") {
                index = skip_comment(&chars, index);
                continue;
            }
            let starts_tag = chars
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, '/' | '!' | '?'));
            if !starts_tag {
                cleaned.push(c);
                index += 1;
                continue;
            }
            in_tag = true;
            tag_buf.clear();
            quote = None;
            index += 1;
            continue;
        }

        cleaned.push(c);
        index += 1;
    }

    let decoded = clean_html_entities(&cleaned);
    decoded.split_whitespace().collect::<Vec<&str>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_main_text_ignores_gt_inside_quoted_attribute() {
        let html = r#"<p><a title="1 > 0">Research reduced latency.</a></p>"#;
        let text = extract_main_text(html);
        assert_eq!(text, "Research reduced latency.");
        assert!(!text.contains("0\">"));
    }

    #[test]
    fn test_extract_main_text_handles_self_closing_br() {
        let html = "<p>First<br/>Second</p>";
        assert_eq!(extract_main_text(html), "First Second");
    }

    #[test]
    fn test_extract_main_text_ignores_lt_inside_script() {
        let html = "<script>if (1 < 2) {}</script><p>Research reduced latency.</p>";
        assert_eq!(extract_main_text(html), "Research reduced latency.");
    }

    #[test]
    fn test_extract_main_text_keeps_similarly_prefixed_tags() {
        let html = "<script-card>Measured throughput increased 30%.</script-card>";
        assert_eq!(
            extract_main_text(html),
            "Measured throughput increased 30%."
        );
    }

    #[test]
    fn test_extract_main_text_ignores_script_tag_inside_attribute() {
        let html = r#"<div data-template="<script>"><p>Latency fell 30%.</p></div>"#;
        assert_eq!(extract_main_text(html), "Latency fell 30%.");
    }

    #[test]
    fn test_extract_main_text_skips_comments_with_apostrophes() {
        let html = "<!-- Publisher's note --><p>Latency fell 30%.</p>";
        assert_eq!(extract_main_text(html), "Latency fell 30%.");
    }

    #[test]
    fn test_extract_main_text_keeps_literal_less_than() {
        let html = "<p>Latency < 10ms, throughput improved 30%.</p>";
        assert_eq!(
            extract_main_text(html),
            "Latency < 10ms, throughput improved 30%."
        );
    }

    #[test]
    fn test_clean_html_entities_decodes_once() {
        assert_eq!(clean_html_entities("&amp;lt;"), "&lt;");
        assert_eq!(clean_html_entities("&amp;amp;"), "&amp;");
        assert_eq!(clean_html_entities("&amp;"), "&");
    }
}