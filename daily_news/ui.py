"""HTML Dashboard rendering for Daily News & Content Discovery Service.

Clean Architecture Layer: Presentation Layer.
Renders self-contained, responsive management UI.
"""

from __future__ import annotations

import html
from typing import Any, Dict, List


def render_dashboard_html(
    config: Dict[str, Any],
    stats: Dict[str, Any],
    candidates: List[Dict[str, Any]],
) -> str:
    """Render modern responsive HTML UI for the Daily News module."""
    module_name = html.escape(config.get("name", "Daily News & Content Discovery"))
    newsnow_url = html.escape(config.get("newsnow_url", "http://127.0.0.1:4444"))
    freshness_h = config.get("freshness_window_hours", 48)
    require_latest = "Yes" if config.get("require_latest_news", True) else "No"
    consumed_count = stats.get("consumed_count", 0)
    cached_count = len(candidates)

    rows = []
    for c in candidates:
        cid = html.escape(c.get("candidate_id", ""))
        title = html.escape(c.get("title", "Untitled"))
        url = html.escape(c.get("canonical_url", "#"))
        pub = html.escape(c.get("publisher", "Unknown"))
        pub_utc = html.escape(c.get("published_at_utc", ""))
        score_info = c.get("scoring", {})
        score = score_info.get("total_score", 0.0)

        rows.append(f"""
        <tr>
          <td><code style="font-size: 0.85em;">{cid}</code></td>
          <td><a href="{url}" target="_blank" rel="noopener noreferrer" style="font-weight: 600; color: #2563eb; text-decoration: none;">{title}</a></td>
          <td><span style="background: #e2e8f0; padding: 2px 6px; border-radius: 4px; font-size: 0.85em;">{pub}</span></td>
          <td style="font-size: 0.85em; color: #64748b;">{pub_utc}</td>
          <td><span style="background: #dcfce7; color: #166534; padding: 2px 8px; border-radius: 9999px; font-weight: bold; font-size: 0.9em;">{score:.2f}</span></td>
        </tr>
        """)

    candidates_table = "".join(rows) if rows else "<tr><td colspan='5' style='text-align: center; color: #94a3b8; padding: 24px;'>No candidates discovered yet. Click Discover News below.</td></tr>"

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{module_name} - Toolcase Gateway</title>
  <style>
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background: #f8fafc;
      color: #0f172a;
      margin: 0;
      padding: 24px;
    }}
    .container {{
      max-width: 1000px;
      margin: 0 auto;
    }}
    .header {{
      background: #ffffff;
      padding: 20px 24px;
      border-radius: 12px;
      box-shadow: 0 1px 3px rgba(0,0,0,0.05);
      margin-bottom: 24px;
      display: flex;
      justify-content: space-between;
      align-items: center;
    }}
    .title-group h1 {{
      margin: 0 0 6px 0;
      font-size: 1.5rem;
      display: flex;
      align-items: center;
      gap: 10px;
    }}
    .stats-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 16px;
      margin-bottom: 24px;
    }}
    .stat-card {{
      background: #ffffff;
      padding: 16px;
      border-radius: 8px;
      box-shadow: 0 1px 2px rgba(0,0,0,0.04);
      border-left: 4px solid #3b82f6;
    }}
    .stat-card .label {{
      font-size: 0.85em;
      color: #64748b;
      margin-bottom: 4px;
    }}
    .stat-card .value {{
      font-size: 1.4em;
      font-weight: 700;
    }}
    .card {{
      background: #ffffff;
      padding: 24px;
      border-radius: 12px;
      box-shadow: 0 1px 3px rgba(0,0,0,0.05);
      margin-bottom: 24px;
    }}
    .card h2 {{
      margin-top: 0;
      font-size: 1.2rem;
      border-bottom: 1px solid #f1f5f9;
      padding-bottom: 12px;
    }}
    table {{
      width: 100%;
      border-collapse: collapse;
      text-align: left;
    }}
    th, td {{
      padding: 10px 12px;
      border-bottom: 1px solid #e2e8f0;
    }}
    th {{
      font-size: 0.85em;
      color: #64748b;
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }}
    .badge {{
      display: inline-block;
      padding: 4px 10px;
      border-radius: 9999px;
      font-size: 0.8rem;
      font-weight: 600;
      background: #ecfdf5;
      color: #059669;
    }}
    .btn {{
      background: #2563eb;
      color: #ffffff;
      border: none;
      padding: 8px 16px;
      border-radius: 6px;
      font-weight: 500;
      cursor: pointer;
    }}
    .btn:hover {{
      background: #1d4ed8;
    }}
  </style>
</head>
<body>
  <div class="container">
    <div class="header">
      <div class="title-group">
        <h1>📰 {module_name}</h1>
        <div style="color: #64748b; font-size: 0.9em;">Clean Architecture Daily Content Ingestion for OpenMontage & Toolcase Gateway</div>
      </div>
      <div>
        <span class="badge">Active & Ready</span>
      </div>
    </div>

    <div class="stats-grid">
      <div class="stat-card">
        <div class="label">Discovered Candidates</div>
        <div class="value">{cached_count}</div>
      </div>
      <div class="stat-card" style="border-left-color: #10b981;">
        <div class="label">Consumed History</div>
        <div class="value">{consumed_count}</div>
      </div>
      <div class="stat-card" style="border-left-color: #8b5cf6;">
        <div class="label">Freshness Window</div>
        <div class="value">{freshness_h}h</div>
      </div>
      <div class="stat-card" style="border-left-color: #f59e0b;">
        <div class="label">Strict Latest News</div>
        <div class="value">{require_latest}</div>
      </div>
    </div>

    <div class="card">
      <h2>Ranked Candidates & Trending Items</h2>
      <table>
        <thead>
          <tr>
            <th>ID</th>
            <th>Title</th>
            <th>Publisher</th>
            <th>Published (UTC)</th>
            <th>Score</th>
          </tr>
        </thead>
        <tbody>
          {candidates_table}
        </tbody>
      </table>
    </div>

    <div class="card">
      <h2>Module Configuration</h2>
      <p style="color: #64748b; font-size: 0.9em;">Configured upstream NewsNow URL: <code>{newsnow_url}</code></p>
      <p style="color: #64748b; font-size: 0.9em;">REST API Endpoints:</p>
      <ul>
        <li><code>GET /custom/daily-news/api/v1/feed</code> &mdash; Ingest and return raw feed JSON</li>
        <li><code>GET /custom/daily-news/api/v1/candidates</code> &mdash; Get normalized and scored candidate list</li>
        <li><code>POST /custom/daily-news/api/v1/discover</code> &mdash; Run priority tiered discovery</li>
        <li><code>GET /custom/daily-news/models</code> &mdash; OpenAI-compatible models endpoint</li>
      </ul>
    </div>
  </div>
</body>
</html>
"""
