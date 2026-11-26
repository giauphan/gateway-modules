//! Interactive HTML Dashboard for Proxy Guard & Upstream Proxy Pool.
//!
//! Clean Architecture: Presentation / Interface Adapters Layer.

use crate::domain::ProxyRotationStrategy;
use crate::ports::{MetricsPort, ProxyPoolManagementPort};
use crate::service::ProxyGuardService;

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

pub fn render_dashboard_html(service: &ProxyGuardService) -> String {
    let metrics = service.get_metrics();
    let proxies = service.list_proxies();
    let strategy = service.get_strategy();

    let strat_str = match strategy {
        ProxyRotationStrategy::RoundRobin => "Round-Robin",
        ProxyRotationStrategy::StickySession => "Sticky Session",
        ProxyRotationStrategy::LowestFailures => "Lowest Failures",
        ProxyRotationStrategy::Random => "Random",
    };

    let total_reqs = metrics.total_requests;
    let blocked_reqs = metrics.blocked_requests;
    let reduction_pct = if total_reqs > 0 {
        (blocked_reqs as f64 / total_reqs as f64) * 100.0
    } else {
        0.0
    };

    let mb_saved = (metrics.bytes_saved as f64) / (1024.0 * 1024.0);
    let mb_upstream = (metrics.upstream_bytes_used as f64) / (1024.0 * 1024.0);
    let cost_saved = metrics.estimated_cost_saved_usd;
    let total_proxies = proxies.len();

    let mut proxy_rows = String::new();
    for p in &proxies {
        let masked = p.to_masked();
        let user_display = masked
            .auth
            .as_ref()
            .map(|a| html_escape(&a.username))
            .unwrap_or_else(|| "-".to_string());
        let country_display = p.country.as_deref().unwrap_or("-");
        let tags_display = if p.tags.is_empty() {
            "-".to_string()
        } else {
            p.tags
                .iter()
                .map(|t| format!("<span class='tag'>{}</span>", html_escape(t)))
                .collect::<Vec<_>>()
                .join(" ")
        };

        let status_badge = if p.failure_count == 0 {
            "<span class='badge ok'>Healthy</span>"
        } else if p.failure_count < 3 {
            "<span class='badge warn'>Recovering</span>"
        } else {
            "<span class='badge err'>Failing</span>"
        };

        proxy_rows.push_str(&format!(
            r#"<tr>
                <td><strong>{}</strong></td>
                <td><span class='proto'>{:?}</span></td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td><span class='success-count'>{}</span> / <span class='fail-count'>{}</span></td>
                <td>{}</td>
                <td><button class='btn-sm btn-del' onclick="removeProxy('{}')">Remove</button></td>
            </tr>"#,
            html_escape(&p.name),
            p.protocol,
            html_escape(&format!("{}:{}", p.host, p.port)),
            user_display,
            html_escape(country_display),
            tags_display,
            p.success_count,
            p.failure_count,
            status_badge,
            html_escape(&p.id),
        ));
    }

    if proxies.is_empty() {
        proxy_rows = "<tr><td colspan='9' style='text-align:center;color:#888;padding:24px;'>No upstream proxies registered in pool. Add proxies using the form below.</td></tr>".to_string();
    }

    // Recent Request Logs
    let recent_logs = service.get_request_logs(25);
    let logs_count = recent_logs.len();
    let mut log_rows = String::new();
    for log in &recent_logs {
        let action_badge = if log.action.starts_with("Blocked") {
            format!(
                "<span class='badge err'>{}</span>",
                html_escape(&log.action)
            )
        } else if log.action.starts_with("Mocked") {
            format!(
                "<span class='badge warn'>{}</span>",
                html_escape(&log.action)
            )
        } else {
            format!("<span class='badge ok'>{}</span>", html_escape(&log.action))
        };

        let saved_str = if log.bytes_saved >= 1024 * 1024 {
            format!("{:.2} MB", log.bytes_saved as f64 / (1024.0 * 1024.0))
        } else if log.bytes_saved > 0 {
            format!("{:.1} KB", log.bytes_saved as f64 / 1024.0)
        } else {
            "-".to_string()
        };

        log_rows.push_str(&format!(
            r#"<tr>
                <td style='white-space:nowrap;font-family:monospace;font-size:12px;color:#8b949e;'>{}</td>
                <td><span class='proto'>{}</span></td>
                <td style='max-width:320px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;' title='{}'><strong>{}</strong><br><span style='font-size:11px;color:#8b949e;'>{}</span></td>
                <td><span class='tag'>{}</span></td>
                <td>{}</td>
                <td><strong style='color:var(--green);'>{}</strong></td>
            </tr>"#,
            html_escape(&log.timestamp),
            html_escape(&log.method),
            html_escape(&log.url),
            html_escape(&log.host),
            html_escape(&log.reason),
            html_escape(&log.category),
            action_badge,
            html_escape(&saved_str),
        ));
    }

    if recent_logs.is_empty() {
        log_rows = "<tr><td colspan='6' style='text-align:center;color:#888;padding:24px;'>No requests intercepted yet. Active traffic will stream here in real time.</td></tr>".to_string();
    }

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Proxy Guard - Multi-Proxy Pool & Traffic Optimizer</title>
<style>
:root {{ --bg: #0d1117; --card: #161b22; --border: #30363d; --text: #c9d1d9; --accent: #58a6ff; --green: #3fb950; --warn: #d29922; --err: #f85149; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: var(--bg); color: var(--text); margin: 0; padding: 24px; }}
.container {{ max-width: 1100px; margin: 0 auto; }}
h1, h2, h3 {{ color: #f0f6fc; margin-top: 0; }}
.header {{ display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--border); padding-bottom: 16px; margin-bottom: 24px; }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 16px; margin-bottom: 24px; }}
.card {{ background: var(--card); border: 1px solid var(--border); border-radius: 8px; padding: 16px; }}
.metric {{ font-size: 26px; font-weight: bold; color: var(--accent); margin-top: 4px; }}
.card-green .metric {{ color: var(--green); }}
table {{ width: 100%; border-collapse: collapse; margin-top: 12px; }}
th, td {{ text-align: left; padding: 10px 12px; border-bottom: 1px solid var(--border); font-size: 14px; }}
th {{ background: #21262d; color: #8b949e; }}
.badge {{ padding: 3px 8px; border-radius: 12px; font-size: 11px; font-weight: 600; text-transform: uppercase; }}
.badge.ok {{ background: rgba(63,185,80,0.15); color: var(--green); }}
.badge.warn {{ background: rgba(210,153,34,0.15); color: var(--warn); }}
.badge.err {{ background: rgba(248,81,73,0.15); color: var(--err); }}
.tag {{ background: #21262d; border: 1px solid var(--border); padding: 2px 6px; border-radius: 4px; font-size: 11px; }}
.proto {{ font-weight: 600; color: #79c0ff; }}
.form-box {{ background: var(--card); border: 1px solid var(--border); border-radius: 8px; padding: 20px; margin-top: 24px; }}
textarea, select, input[type=text] {{ width: 100%; box-sizing: border-box; background: #0d1117; border: 1px solid var(--border); color: #fff; padding: 10px; border-radius: 6px; font-family: monospace; font-size: 13px; margin-bottom: 12px; }}
textarea {{ min-height: 100px; resize: vertical; }}
.btn {{ background: #238636; color: #fff; border: 0; padding: 8px 16px; border-radius: 6px; cursor: pointer; font-weight: 600; }}
.btn:hover {{ background: #2ea043; }}
.btn-sm {{ padding: 4px 8px; font-size: 12px; }}
.btn-del {{ background: transparent; color: var(--err); border: 1px solid var(--err); }}
.btn-del:hover {{ background: var(--err); color: #fff; }}
.success-count {{ color: var(--green); font-weight: 600; }}
.fail-count {{ color: var(--err); font-weight: 600; }}
</style>
</head>
<body>
<div class="container">
  <div class="header">
    <div>
      <h1>🛡️ Proxy Guard Dashboard</h1>
      <div style="color:#8b949e;font-size:14px;">Multi-Upstream Proxy Pool, Live Reduction Stream &amp; Cost Optimizer</div>
    </div>
    <div>
      <button class="btn btn-sm" onclick="location.reload()">🔄 Refresh</button>
    </div>
  </div>

  <div class="grid">
    <div class="card">
      <div style="font-size:13px;color:#8b949e;">Active Proxies</div>
      <div class="metric">{total_proxies}</div>
    </div>
    <div class="card">
      <div style="font-size:13px;color:#8b949e;">Rotation Strategy</div>
      <div class="metric" style="font-size:20px;padding-top:6px;">{strat_str}</div>
    </div>
    <div class="card card-green">
      <div style="font-size:13px;color:#8b949e;">Request Reduction</div>
      <div class="metric">{reduction_pct:.1}%</div>
      <div style="font-size:12px;color:#8b949e;margin-top:2px;">{blocked_reqs} blocked / {total_reqs} total</div>
    </div>
    <div class="card card-green">
      <div style="font-size:13px;color:#8b949e;">Bandwidth Saved</div>
      <div class="metric">{mb_saved:.2} MB</div>
    </div>
    <div class="card">
      <div style="font-size:13px;color:#8b949e;">Upstream Used</div>
      <div class="metric" style="color:#79c0ff;">{mb_upstream:.2} MB</div>
    </div>
    <div class="card card-green">
      <div style="font-size:13px;color:#8b949e;">Cost Saved (USD)</div>
      <div class="metric">${cost_saved:.4}</div>
    </div>
  </div>

  <div class="card" style="padding:0;overflow-x:auto;">
    <div style="padding:16px 20px;border-bottom:1px solid var(--border);display:flex;justify-content:space-between;align-items:center;">
      <h3 style="margin:0;">Registered Upstream Proxies</h3>
      <div style="font-size:13px;color:#8b949e;">Strategy: <strong>{strat_str}</strong></div>
    </div>
    <table>
      <thead>
        <tr>
          <th>Name</th>
          <th>Protocol</th>
          <th>Host:Port</th>
          <th>User</th>
          <th>Country</th>
          <th>Tags</th>
          <th>Success / Fail</th>
          <th>Status</th>
          <th>Action</th>
        </tr>
      </thead>
      <tbody>
        {proxy_rows}
      </tbody>
    </table>
  </div>

  <div class="card" style="padding:0;overflow-x:auto;margin-top:24px;">
    <div style="padding:16px 20px;border-bottom:1px solid var(--border);display:flex;justify-content:space-between;align-items:center;">
      <div>
        <h3 style="margin:0;">📋 Live Intercepted Request Logs</h3>
        <div style="font-size:12px;color:#8b949e;margin-top:2px;">Audit trail of intercepted requests showing real-time bandwidth reduction (last {logs_count})</div>
      </div>
      <div>
        <button class="btn btn-sm btn-del" onclick="clearLogs()">Clear Logs</button>
      </div>
    </div>
    <table>
      <thead>
        <tr>
          <th>Time</th>
          <th>Method</th>
          <th>Host &amp; URL</th>
          <th>Category</th>
          <th>Action</th>
          <th>Data Saved</th>
        </tr>
      </thead>
      <tbody>
        {log_rows}
      </tbody>
    </table>
  </div>

  <div class="form-box">
    <h3>➕ Add Multiple Upstream Proxies</h3>
    <p style="font-size:13px;color:#8b949e;margin-top:0;">Enter one unique proxy per line using <code>http://username:password@host:port</code> (or <code>socks5://username:password@host:port</code>) and set the 2-letter Country Acronym field below.</p>
    <textarea id="proxyLines" placeholder="http://username:password@host:port&#10;socks5://username:password@host:port"></textarea>
    <div style="display:flex;gap:12px;margin-bottom:12px;align-items:center;flex-wrap:wrap;">
      <input type="text" id="countryCode" placeholder="Country (e.g. VN, US, or -cc-vn)" style="margin:0;max-width:250px;">
      <input type="text" id="proxyTag" placeholder="Optional Tag (e.g. residential, mobile)" style="margin:0;max-width:250px;">
      <label style="font-size:13px;color:#8b949e;display:flex;align-items:center;gap:6px;cursor:pointer;">
        <input type="checkbox" id="appendCc" style="width:auto;margin:0;"> Append <code>-cc-&lt;country&gt;</code> to username
      </label>
    </div>
    <button class="btn" onclick="submitMultiProxy()">Add to Proxy Pool</button>
  </div>
</div>

<script>
function getApiEndpoint(subpath) {{
  const p = window.location.pathname;
  if (p.includes('/custom/proxy-guard')) {{
    return '/custom/proxy-guard' + subpath;
  }}
  if (p.includes('/modules/proxy-guard')) {{
    return '/modules/proxy-guard' + subpath;
  }}
  return subpath;
}}

async function submitMultiProxy() {{
  const raw = document.getElementById('proxyLines').value.trim();
  if (!raw) {{ alert('Please enter at least one proxy URL'); return; }}
  const lines = raw.split('\n').map(l => l.trim()).filter(l => l.length > 0);
  let country = document.getElementById('countryCode').value.trim() || undefined;
  const appendCc = document.getElementById('appendCc').checked;
  if (country && appendCc && !country.toLowerCase().startsWith('-cc-') && !country.toLowerCase().startsWith('_cc_')) {{
    country = '-cc-' + country.toLowerCase();
  }}
  const tag = document.getElementById('proxyTag').value.trim() || undefined;
  const tags = tag ? [tag] : undefined;

  const proxies = lines.map(url => ({{ url, country, tags }}));
  const res = await fetch(getApiEndpoint('/proxies/add'), {{
    method: 'POST',
    headers: {{ 'Content-Type': 'application/json' }},
    body: JSON.stringify({{ proxies }})
  }});
  const data = await res.json();
  if (data.status === 'ok') {{
    alert(`Successfully added ${{data.added_count}} proxies to pool!`);
    location.reload();
  }} else {{
    alert('Error adding proxies: ' + JSON.stringify(data));
  }}
}}

async function removeProxy(id) {{
  if (!confirm(`Remove proxy ${{id}} from pool?`)) return;
  const res = await fetch(getApiEndpoint('/proxies/remove'), {{
    method: 'POST',
    headers: {{ 'Content-Type': 'application/json' }},
    body: JSON.stringify({{ id }})
  }});
  const data = await res.json();
  if (data.status === 'ok') {{
    location.reload();
  }} else {{
    alert('Failed to remove proxy: ' + (data.error || 'Unknown error'));
  }}
}}

async function clearLogs() {{
  if (!confirm('Clear recent intercepted request logs?')) return;
  const res = await fetch(getApiEndpoint('/logs/clear'), {{
    method: 'POST'
  }});
  const data = await res.json();
  if (data.status === 'ok') {{
    location.reload();
  }}
}}
</script>
</body>
</html>"##
    )
}
