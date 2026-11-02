//! Interactive HTML dashboard presenter for Daily News.
//!
//! Renders an interactive responsive web dashboard presenting
//! operational statistics, configuration, and recent candidates.

use crate::domain::{ModuleConfig, NormalizedCandidate};

pub fn render_dashboard_html(
    version: &str,
    engine: &str,
    config: &ModuleConfig,
    consumed_count: usize,
    candidates: &[NormalizedCandidate],
) -> String {
    let mut candidate_rows = String::new();
    for (i, c) in candidates.iter().enumerate() {
        let score = c.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
        let disc_link = match &c.discussion_url {
            Some(d) if d != &c.canonical_url => {
                format!(r#"<a href="{d}" target="_blank" class="badge">Discussion</a>"#)
            }
            _ => String::new(),
        };
        candidate_rows.push_str(&format!(
            r#"<tr>
                <td>{}</td>
                <td>
                    <strong><a href="{}" target="_blank">{}</a></strong>
                    {}
                </td>
                <td><span class="tag">{}</span></td>
                <td><span class="score">{:.2}</span></td>
                <td>{}</td>
            </tr>"#,
            i + 1,
            c.canonical_url,
            c.title,
            disc_link,
            c.publisher,
            score,
            c.published_at_utc
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
            <h1>📰 Daily News & Content Discovery Service <span class="badge-engine">{}</span></h1>
            <div>Version {}</div>
        </header>

        <div class="grid">
            <div class="card">
                <div class="card-label">Runtime Engine</div>
                <div class="card-value" style="font-size: 1.25rem;">{}</div>
            </div>
            <div class="card">
                <div class="card-label">Consumed Items</div>
                <div class="card-value">{}</div>
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
                {}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        engine,
        version,
        engine,
        consumed_count,
        config.freshness_window_hours,
        candidates.len(),
        candidate_rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_dashboard() {
        let cfg = ModuleConfig::default();
        let html = render_dashboard_html("1.3.0", "rust-native", &cfg, 5, &[]);
        assert!(html.contains("Daily News"));
        assert!(html.contains("rust-native"));
        assert!(html.contains("Consumed Items"));
    }
}
