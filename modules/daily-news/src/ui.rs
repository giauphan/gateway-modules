//! Interactive HTML dashboard presenter for Daily News.
//!
//! Renders an interactive responsive web dashboard presenting
//! operational statistics, configuration, and recent candidates.

use crate::domain::{ModuleConfig, NormalizedCandidate};

/// Context-aware HTML entity escaping for text and attribute contexts.
pub fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// URL sanitizer restricting links to safe HTTP and HTTPS schemes.
/// Defangs dangerous schemes such as `javascript:`, `data:`, and `vbscript:`.
pub fn sanitize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        html_escape(trimmed)
    } else {
        "#".to_string()
    }
}

pub fn render_dashboard_html(
    version: &str,
    engine: &str,
    config: &ModuleConfig,
    consumed_count: usize,
    candidates: &[NormalizedCandidate],
) -> String {
    let safe_version = html_escape(version);
    let safe_engine = html_escape(engine);

    let mut candidate_rows = String::new();
    for (i, c) in candidates.iter().enumerate() {
        let score = c.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
        let safe_url = sanitize_url(&c.canonical_url);
        let safe_title = html_escape(&c.title);
        let safe_publisher = html_escape(&c.publisher);
        let safe_pub_time = html_escape(&c.published_at_utc);

        let disc_link = match &c.discussion_url {
            Some(d) if d != &c.canonical_url => {
                let safe_disc_url = sanitize_url(d);
                format!(r#"<a href="{safe_disc_url}" target="_blank" rel="noopener noreferrer" class="badge">Discussion</a>"#)
            }
            _ => String::new(),
        };

        candidate_rows.push_str(&format!(
            r#"<tr>
                <td>{}</td>
                <td>
                    <strong><a href="{safe_url}" target="_blank" rel="noopener noreferrer">{safe_title}</a></strong>
                    {disc_link}
                </td>
                <td><span class="tag">{safe_publisher}</span></td>
                <td><span class="score">{:.2}</span></td>
                <td>{safe_pub_time}</td>
            </tr>"#,
            i + 1,
            score
        ));
    }

    if candidate_rows.is_empty() {
        candidate_rows = r#"<tr><td colspan="5" style="text-align:center; padding: 2rem;">No candidates currently cached. Run discovery to refresh feeds.</td></tr>"#.to_string();
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Daily News & Content Discovery Service</title>
    <style>
        :root {{
            --bg: #0f172a;
            --surface: #1e293b;
            --border: #334155;
            --primary: #38bdf8;
            --text: #f8fafc;
            --muted: #94a3b8;
            --accent: #22c55e;
        }}
        body {{
            margin: 0;
            padding: 2rem;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            background: var(--bg);
            color: var(--text);
        }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        header {{ display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--border); padding-bottom: 1rem; margin-bottom: 2rem; }}
        h1 {{ margin: 0; font-size: 1.5rem; display: flex; align-items: center; gap: 0.5rem; }}
        .badge-engine {{ background: #0284c7; color: white; padding: 0.25rem 0.6rem; border-radius: 9999px; font-size: 0.75rem; text-transform: uppercase; font-weight: 600; }}
        .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 1rem; margin-bottom: 2rem; }}
        .card {{ background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 1.25rem; }}
        .card-label {{ font-size: 0.8rem; color: var(--muted); text-transform: uppercase; letter-spacing: 0.05em; }}
        .card-value {{ font-size: 1.75rem; font-weight: 700; margin-top: 0.5rem; color: var(--primary); }}
        table {{ width: 100%; border-collapse: collapse; background: var(--surface); border-radius: 8px; overflow: hidden; border: 1px solid var(--border); }}
        th, td {{ padding: 0.75rem 1rem; text-align: left; border-bottom: 1px solid var(--border); }}
        th {{ background: #182234; font-size: 0.8rem; text-transform: uppercase; color: var(--muted); }}
        a {{ color: var(--primary); text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
        .score {{ font-weight: bold; color: var(--accent); }}
        .badge {{ font-size: 0.7rem; background: var(--border); color: var(--text); padding: 0.15rem 0.4rem; border-radius: 4px; margin-left: 0.5rem; }}
        .tag {{ background: #334155; padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.75rem; }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>📰 Daily News & Content Discovery Service <span class="badge-engine">{safe_engine}</span></h1>
            <div style="display: flex; align-items: center; gap: 1rem;">
                <a href="/flow" target="_blank" rel="noopener noreferrer" class="badge-engine" style="text-decoration: none; padding: 0.3rem 0.75rem;">Interactive Flowchart ➔</a>
                <div>Version {safe_version}</div>
            </div>
        </header>

        <div class="grid">
            <div class="card">
                <div class="card-label">Runtime Engine</div>
                <div class="card-value" style="font-size: 1.25rem;">{safe_engine}</div>
            </div>
            <div class="card">
                <div class="card-label">Consumed Items</div>
                <div class="card-value">{consumed_count}</div>
            </div>
            <div class="card">
                <div class="card-label">Freshness Window</div>
                <div class="card-value">{}h</div>
            </div>
            <div class="card">
                <div class="card-label">Cached Candidates</div>
                <div class="card-value">{}</div>
            </div>
        </div>

        <h2>Ranked Content Candidates</h2>
        <table>
            <thead>
                <tr>
                    <th style="width: 40px;">#</th>
                    <th>Title & Destination</th>
                    <th>Publisher</th>
                    <th>Score</th>
                    <th>Published (UTC)</th>
                </tr>
            </thead>
            <tbody>
                {candidate_rows}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        config.freshness_window_hours,
        candidates.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RawProvenance;

    #[test]
    fn test_render_dashboard() {
        let cfg = ModuleConfig::default();
        let html = render_dashboard_html(env!("CARGO_PKG_VERSION"), "rust-native", &cfg, 5, &[]);
        assert!(html.contains("Daily News"));
        assert!(html.contains("rust-native"));
        assert!(html.contains("Consumed Items"));
    }

    #[test]
    fn test_dashboard_xss_mitigation() {
        let cfg = ModuleConfig::default();
        let malicious_candidate = NormalizedCandidate {
            candidate_id: "c_xss".to_string(),
            canonical_url: "javascript:alert(document.domain)".to_string(),
            original_url: "javascript:alert(document.domain)".to_string(),
            url: "javascript:alert(document.domain)".to_string(),
            source_url: "javascript:alert(document.domain)".to_string(),
            source_type: "news".to_string(),
            title: r#"Evil <script>alert('xss')</script> & "quotes""#.to_string(),
            publisher: r#"Attacker <img src=x onerror=alert(1)>"#.to_string(),
            normalized_publisher: "attacker".to_string(),
            published_at_utc: r#"2026-11-04" onmouseover="alert(1)"#.to_string(),
            published_at: "2026-11-04".to_string(),
            content_hash: "hash".to_string(),
            title_hash: "thash".to_string(),
            raw_provenance: RawProvenance::default(),
            adapter_version: "1.8.0".to_string(),
            topic_family: "unclassified".to_string(),
            author: None,
            description: None,
            body: None,
            summary: None,
            tags: vec![],
            language: "en".to_string(),
            image_url: None,
            is_evergreen: false,
            content_kind: "news".to_string(),
            updated_at: None,
            discussion_url: Some("javascript:alert('discussion_xss')".to_string()),
            evidence: Default::default(),
            independent_fact_check: None,
            scoring: None,
            content_origin: Default::default(),
            published_at_kind: "unknown".to_string(),
            research: Default::default(),
        };

        let html = render_dashboard_html("1.8.0", "rust-native", &cfg, 1, &[malicious_candidate]);

        // Unescaped HTML tags and javascript hrefs MUST NOT exist in rendered HTML
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(!html.contains("onmouseover=\"alert(1)\""));

        // Must contain sanitized and escaped counterparts
        assert!(html.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
        assert!(html.contains("href=\"#\""));
        assert!(html.contains("2026-11-04&quot; onmouseover=&quot;alert(1)"));
    }
}
