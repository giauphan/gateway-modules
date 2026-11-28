//! Interactive HTML Dashboard for Proxy Guard & Upstream Proxy Pool.
//!
//! Clean Architecture: Presentation / Interface Adapters Layer.

mod charts;
mod scripts;
mod styles;

use crate::domain::{DefaultRoute, ProxyRotationStrategy, Rule, RuleCategory, UpstreamProxy};
use crate::ports::{MetricsPort, ProxyPoolManagementPort, RuleManagementPort};
use crate::service::ProxyGuardService;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn category_enabled(rules: &[Rule], cat: RuleCategory) -> bool {
    rules.iter().any(|r| r.category == cat && r.enabled)
}

fn default_route_label(route: &DefaultRoute) -> String {
    match route {
        DefaultRoute::ForwardUpstream => "Forward Upstream".to_string(),
        DefaultRoute::Direct => "Direct Bypass".to_string(),
        DefaultRoute::Pool(name) => format!("Pool: {name}"),
    }
}

fn render_proxy_rows(proxies: &[UpstreamProxy], now_secs: u64) -> String {
    if proxies.is_empty() {
        return "<tr><td colspan='10' style='text-align:center;color:#888;padding:24px;'>No upstream proxies registered in pool. Add proxies using the form below.</td></tr>".to_string();
    }

    let mut proxy_rows = String::with_capacity(proxies.len() * 420);
    for p in proxies {
        let masked = p.to_masked();
        let user_display = masked
            .auth
            .as_ref()
            .map(|a| html_escape(&a.username))
            .unwrap_or_else(|| "-".to_string());
        let country_display = p.country.as_deref().unwrap_or("-");
        let pool_display = p.pool_name.as_deref().unwrap_or("default");
        let tags_display = if p.tags.is_empty() {
            "-".to_string()
        } else {
            p.tags
                .iter()
                .map(|t| format!("<span class='tag'>{}</span>", html_escape(t)))
                .collect::<Vec<_>>()
                .join(" ")
        };

        let is_quarantined = p.is_auth_quarantined(now_secs);
        let status_badge = if is_quarantined {
            let rem = p
                .quarantined_until_epoch_secs
                .unwrap_or(now_secs)
                .saturating_sub(now_secs);
            format!("<span class='badge quarantine'>407 Quarantined ({rem}s)</span>")
        } else if p.failure_count == 0 {
            "<span class='badge ok'>Healthy</span>".to_string()
        } else if p.failure_count < 3 {
            "<span class='badge warn'>Recovering</span>".to_string()
        } else {
            "<span class='badge err'>Failing</span>".to_string()
        };

        let escaped_id = html_escape(&p.id);
        let restore_btn = if is_quarantined || p.failure_count > 0 {
            format!(
                "<button class='btn-sm btn-restore' data-proxy-id='{escaped_id}' onclick='unquarantineProxy(this.dataset.proxyId)' title='Clear 407 quarantine & reset failure count'>Restore</button> "
            )
        } else {
            String::new()
        };

        proxy_rows.push_str(&format!(
            r#"<tr>
                <td><strong>{}</strong></td>
                <td><span class='badge info'>{}</span></td>
                <td><span class='proto'>{:?}</span></td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td><span class='success-count'>{}</span> / <span class='fail-count'>{}</span></td>
                <td>{}</td>
                <td style='white-space:nowrap;'>{}<button class='btn-sm btn-del' data-proxy-id='{}' onclick='removeProxy(this.dataset.proxyId)'>Remove</button></td>
            </tr>"#,
            html_escape(&p.name),
            html_escape(pool_display),
            p.protocol,
            html_escape(&format!("{}:{}", p.host, p.port)),
            user_display,
            html_escape(country_display),
            tags_display,
            p.success_count,
            p.failure_count,
            status_badge,
            restore_btn,
            escaped_id,
        ));
    }
    proxy_rows
}

fn render_log_rows(service: &ProxyGuardService) -> (usize, String) {
    let recent_logs = service.get_request_logs(30);
    let logs_count = recent_logs.len();
    if recent_logs.is_empty() {
        return (
            0,
            "<tr><td colspan='6' style='text-align:center;color:#888;padding:24px;'>No requests intercepted yet. Active traffic will stream here in real time.</td></tr>".to_string(),
        );
    }

    let mut log_rows = String::with_capacity(recent_logs.len() * 340);
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
        } else if log.action.contains("Direct") {
            format!(
                "<span class='badge info'>{}</span>",
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
            r#"<tr data-log-row="1">
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

    (logs_count, log_rows)
}

pub fn render_dashboard_html(service: &ProxyGuardService) -> String {
    let now_secs = current_epoch_secs();
    let metrics = service.get_metrics();
    let proxies = service.list_proxies();
    let strategy = service.get_strategy();
    let default_route = service.get_default_route();
    let rules = service.get_rules();

    let strat_str = match strategy {
        ProxyRotationStrategy::RoundRobin => "Round-Robin",
        ProxyRotationStrategy::StickySession => "Sticky Session",
        ProxyRotationStrategy::LowestFailures => "Lowest Failures",
        ProxyRotationStrategy::Random => "Random",
    };

    let default_route_str = html_escape(&default_route_label(&default_route));
    let is_direct_default = matches!(default_route, DefaultRoute::Direct);
    let is_upstream_default = matches!(default_route, DefaultRoute::ForwardUpstream);

    let total_reqs = metrics.total_requests;
    let blocked_reqs = metrics.blocked_requests;
    let reduction_pct = if total_reqs > 0 {
        (blocked_reqs as f64 / total_reqs as f64) * 100.0
    } else {
        0.0
    };

    let mb_saved = (metrics.bytes_saved as f64) / 1_048_576.0;
    let mb_upstream = (metrics.upstream_bytes_used as f64) / 1_048_576.0;
    let gb_saved = (metrics.bytes_saved as f64) / 1_073_741_824.0;
    let gb_upstream = (metrics.upstream_bytes_used as f64) / 1_073_741_824.0;
    let total_data_bytes = metrics
        .bytes_saved
        .saturating_add(metrics.upstream_bytes_used);
    let gb_total = (total_data_bytes as f64) / 1_073_741_824.0;
    let data_opt_pct = if total_data_bytes > 0 {
        (metrics.bytes_saved as f64 / total_data_bytes as f64) * 100.0
    } else {
        0.0
    };

    let cost_per_gb = metrics.cost_per_gb_usd;
    let cost_saved = metrics.estimated_cost_saved_usd;
    let total_proxies = proxies.len();
    let quarantined_count = proxies
        .iter()
        .filter(|p| p.is_auth_quarantined(now_secs))
        .count();

    let charts_html = charts::render_charts_section(&metrics, &proxies, now_secs);
    let proxy_rows = render_proxy_rows(&proxies, now_secs);
    let (logs_count, log_rows) = render_log_rows(service);

    let check = |cond: bool| {
        if cond {
            "<span class='menu-check'>✓</span>"
        } else {
            ""
        }
    };
    let checked_attr = |cond: bool| if cond { "checked" } else { "" };

    let chk_analytics = checked_attr(category_enabled(&rules, RuleCategory::Analytics));
    let chk_ads = checked_attr(category_enabled(&rules, RuleCategory::Advertising));
    let chk_telemetry = checked_attr(category_enabled(&rules, RuleCategory::Telemetry));
    let chk_media = checked_attr(category_enabled(&rules, RuleCategory::HeavyMedia));
    let chk_fonts = checked_attr(category_enabled(&rules, RuleCategory::Font));

    let version = env!("CARGO_PKG_VERSION");
    let css = styles::DASHBOARD_CSS;
    let js = scripts::DASHBOARD_JS;

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Proxy Guard - Multi-Proxy Pool &amp; Traffic Optimizer</title>
<style>{css}</style>
</head>
<body>
<header class="topbar">
  <div class="topbar-inner">
    <div class="brand">
      <h1>🛡️ Proxy Guard Dashboard</h1>
      <span class="ver-pill">v{version}</span>
    </div>
    <nav class="nav-links" aria-label="Dashboard sections">
      <a class="nav-link" href="#overview">Overview</a>
      <a class="nav-link" href="#charts">Charts</a>
      <a class="nav-link" href="#proxies">Proxy Pool</a>
      <a class="nav-link" href="#routing">Split Routing</a>
      <a class="nav-link" href="#logs">Live Logs</a>
      <a class="nav-link" href="#add-proxies">Add Proxies</a>
    </nav>
    <div class="toolbar-actions">
      <div class="action-menu" id="actionMenu">
        <button class="btn btn-secondary btn-sm" onclick="toggleActionMenu(event)" aria-haspopup="true">⚡ Actions ▾</button>
        <div class="action-menu-panel" role="menu">
          <div class="menu-section">Rotation Strategy</div>
          <button class="menu-item" onclick="setStrategy('round_robin')"><span>Round-Robin</span>{rr_chk}</button>
          <button class="menu-item" onclick="setStrategy('lowest_failures')"><span>Lowest Failures</span>{lf_chk}</button>
          <button class="menu-item" onclick="setStrategy('sticky_session')"><span>Sticky Session</span>{ss_chk}</button>
          <button class="menu-item" onclick="setStrategy('random')"><span>Random</span>{rn_chk}</button>
          <div class="menu-divider"></div>
          <div class="menu-section">Default Split Route</div>
          <button class="menu-item" onclick="setDefaultRoute('forward_upstream')"><span>Forward Upstream</span>{up_chk}</button>
          <button class="menu-item" onclick="setDefaultRoute('direct')"><span>Direct Bypass (0 GB)</span>{dr_chk}</button>
          <div class="menu-divider"></div>
          <div class="menu-section">Dashboard &amp; Data</div>
          <button class="menu-item" onclick="toggleAutoRefresh()"><span>Auto-Refresh</span><span id="autoRefreshState" class="muted">OFF</span></button>
          <button class="menu-item" onclick="exportMetricsJson()"><span>Export Metrics JSON</span><span>↓</span></button>
          <div class="menu-divider"></div>
          <button class="menu-item danger" onclick="resetMetrics()"><span>Reset Cumulative Metrics</span><span>↺</span></button>
          <button class="menu-item danger" onclick="clearLogs()"><span>Clear Request Logs</span><span>✕</span></button>
        </div>
      </div>
      <button class="btn btn-sm" onclick="location.reload()">🔄 Refresh</button>
    </div>
  </div>
</header>

<div class="container">
  <section id="overview" class="grid">
    <div class="card">
      <div class="card-sub" style="margin-top:0;">Active Proxies</div>
      <div class="metric">{total_proxies}</div>
      <div class="card-sub">{quarantined_count} in 407 quarantine</div>
    </div>
    <div class="card">
      <div class="card-sub" style="margin-top:0;">Strategy / Default Route</div>
      <div class="metric" style="font-size:18px;padding-top:4px;">{strat_str}</div>
      <div class="card-sub">Default: {default_route_str}</div>
    </div>
    <div class="card card-green">
      <div class="card-sub" style="margin-top:0;">Request Reduction</div>
      <div class="metric">{reduction_pct:.1}%</div>
      <div class="card-sub">{blocked_reqs} blocked / {total_reqs} total</div>
    </div>
    <div class="card card-green">
      <div class="card-sub" style="margin-top:0;">Data Optimized (GB Saved)</div>
      <div class="metric">{gb_saved:.4} GB</div>
      <div class="card-sub">{mb_saved:.2} MB saved ({data_opt_pct:.1}% of {gb_total:.4} GB)</div>
    </div>
    <div class="card">
      <div class="card-sub" style="margin-top:0;">Upstream Used (GB)</div>
      <div class="metric" style="color:#79c0ff;">{gb_upstream:.4} GB</div>
      <div class="card-sub">{mb_upstream:.2} MB forwarded &middot; {cache_hits} cache hits</div>
    </div>
    <div class="card card-green">
      <div class="card-sub" style="margin-top:0;">Cost Saved (USD)</div>
      <div class="metric">${cost_saved:.4}</div>
      <div class="card-sub">@${cost_per_gb:.2} / GB</div>
    </div>
  </section>

  <div class="card" style="margin-bottom:20px;">
    <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:8px;gap:12px;flex-wrap:wrap;">
      <strong style="color:#f0f6fc;">📊 Data Optimization Efficiency</strong>
      <span style="font-size:13px;color:var(--green);font-weight:600;">{data_opt_pct:.1}% Optimized ({gb_saved:.4} GB saved / {gb_total:.4} GB total)</span>
    </div>
    <div class="bar-track" style="height:10px;">
      <div class="bar-seg" style="width:{data_opt_pct:.1}%;background:linear-gradient(90deg,#238636,#3fb950);"></div>
    </div>
  </div>

  {charts_html}

  <section id="proxies" class="card card-table">
    <div class="card-header">
      <h3 style="margin:0;">Registered Upstream Proxies</h3>
      <div class="row-flex">
        <span class="muted" style="font-size:13px;">Strategy: <strong>{strat_str}</strong></span>
        <select aria-label="Rotation strategy" onchange="setStrategy(this.value)" style="width:auto;padding:4px 8px;font-size:12px;">
          <option value="round_robin" {rr_sel}>Round-Robin</option>
          <option value="lowest_failures" {lf_sel}>Lowest Failures</option>
          <option value="sticky_session" {ss_sel}>Sticky Session</option>
          <option value="random" {rn_sel}>Random</option>
        </select>
      </div>
    </div>
    <table>
      <thead>
        <tr>
          <th>Name</th>
          <th>Pool</th>
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
      <tbody>{proxy_rows}</tbody>
    </table>
  </section>

  <section id="routing" class="form-box">
    <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:12px;flex-wrap:wrap;gap:10px;">
      <h3 style="margin:0;">🔀 Split-Routing &amp; Filter Rules</h3>
      <span class="muted" style="font-size:12px;">Default fallback: <strong>{default_route_str}</strong></span>
    </div>
    <div class="row-flex" style="margin-bottom:14px;">
      <label class="toggle-chip"><input type="checkbox" {chk_analytics} onchange="toggleCategory('block_analytics', this.checked)"> Block Analytics</label>
      <label class="toggle-chip"><input type="checkbox" {chk_ads} onchange="toggleCategory('block_ads', this.checked)"> Block Ads</label>
      <label class="toggle-chip"><input type="checkbox" {chk_telemetry} onchange="toggleCategory('block_telemetry', this.checked)"> Block Telemetry</label>
      <label class="toggle-chip"><input type="checkbox" {chk_media} onchange="toggleCategory('block_media', this.checked)"> Block Heavy Media</label>
      <label class="toggle-chip"><input type="checkbox" {chk_fonts} onchange="toggleCategory('block_fonts', this.checked)"> Block Fonts</label>
    </div>
    <div class="row-flex">
      <input type="text" id="ruleDomain" placeholder="Domain (e.g. services.sheerid.com or cdn.example.com)" style="flex:2;min-width:220px;">
      <select id="ruleAction" onchange="toggleRulePoolInput()" style="flex:1;min-width:170px;">
        <option value="pool">Route to Pool</option>
        <option value="direct">Direct Bypass (0 GB)</option>
        <option value="block">Block (Reject)</option>
        <option value="whitelist">Whitelist (Upstream)</option>
      </select>
      <input type="text" id="rulePool" placeholder="Pool name (e.g. residential)" style="flex:1;min-width:160px;">
      <button class="btn btn-sm" onclick="submitQuickRule()">Save Rule</button>
    </div>
  </section>

  <section id="logs" class="card card-table">
    <div class="card-header">
      <div>
        <h3 style="margin:0;">📋 Live Intercepted Request Logs</h3>
        <div class="card-sub">Audit trail of intercepted requests showing real-time bandwidth reduction (last {logs_count})</div>
      </div>
      <div class="row-flex">
        <input type="text" id="logSearch" oninput="filterLogs()" placeholder="Filter host / action..." style="width:200px;padding:5px 8px;font-size:12px;">
        <button class="btn btn-sm btn-del" onclick="clearLogs()">Clear Logs</button>
      </div>
    </div>
    <table id="logsTable">
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
      <tbody>{log_rows}</tbody>
    </table>
  </section>

  <section id="add-proxies" class="form-box">
    <h3>➕ Add Multiple Upstream Proxies</h3>
    <p class="muted" style="font-size:13px;margin-top:0;">Enter one unique proxy per line using <code>http://username:password@host:port</code> (or <code>socks5://username:password@host:port</code>).</p>
    <textarea id="proxyLines" placeholder="http://username:password@host:port&#10;socks5://username:password@host:port"></textarea>
    <div class="row-flex" style="margin-bottom:12px;">
      <input type="text" id="countryCode" value="VN" placeholder="Country (e.g. VN, US)" style="max-width:180px;">
      <input type="text" id="proxyPoolName" placeholder="Pool (e.g. residential, static_isp_us)" style="max-width:230px;">
      <input type="text" id="proxyTag" placeholder="Tag (e.g. residential, mobile)" style="max-width:200px;">
      <label class="muted" style="font-size:13px;display:flex;align-items:center;gap:6px;cursor:pointer;">
        <input type="checkbox" id="appendCc" checked style="width:auto;margin:0;"> Append <code>-cc-&lt;country&gt;</code>
      </label>
    </div>
    <button class="btn" onclick="submitMultiProxy()">Add to Proxy Pool</button>
  </section>
</div>

<div id="toast" role="status" aria-live="polite"></div>
<script>{js}</script>
</body>
</html>"##,
        rr_chk = check(strategy == ProxyRotationStrategy::RoundRobin),
        lf_chk = check(strategy == ProxyRotationStrategy::LowestFailures),
        ss_chk = check(strategy == ProxyRotationStrategy::StickySession),
        rn_chk = check(strategy == ProxyRotationStrategy::Random),
        up_chk = check(is_upstream_default),
        dr_chk = check(is_direct_default),
        rr_sel = if strategy == ProxyRotationStrategy::RoundRobin {
            "selected"
        } else {
            ""
        },
        lf_sel = if strategy == ProxyRotationStrategy::LowestFailures {
            "selected"
        } else {
            ""
        },
        ss_sel = if strategy == ProxyRotationStrategy::StickySession {
            "selected"
        } else {
            ""
        },
        rn_sel = if strategy == ProxyRotationStrategy::Random {
            "selected"
        } else {
            ""
        },
        cache_hits = metrics.cache_hits,
    )
}
