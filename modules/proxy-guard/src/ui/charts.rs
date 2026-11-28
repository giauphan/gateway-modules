//! Pure-SVG Dashboard Charts for Proxy Guard.
//!
//! Clean Architecture: Presentation / Interface Adapters Layer.
//! Renders zero-dependency inline vector charts so the dashboard consumes 0 external bytes.

use crate::domain::{FlowMetrics, UpstreamProxy};
use crate::ui::html_escape;

/// Renders the 2x2 interactive-style SVG charts grid:
/// 1. Request Disposition Donut Chart (Blocked / Direct / Upstream / Cached)
/// 2. Bandwidth Comparison Bar Chart (Saved vs. Upstream TX/RX + Cache Saved)
/// 3. Per-Pool Traffic & Bandwidth Breakdown
/// 4. Proxy Pool Health & HTTP 407 Quarantine Breakdown
pub fn render_charts_section(
    metrics: &FlowMetrics,
    proxies: &[UpstreamProxy],
    now_secs: u64,
) -> String {
    let disposition_card = render_disposition_donut(metrics);
    let bandwidth_card = render_bandwidth_chart(metrics);
    let pool_card = render_pool_traffic_chart(metrics);
    let health_card = render_proxy_health_chart(proxies, now_secs);

    format!(
        r#"<section id="charts" class="charts-grid">
  {disposition_card}
  {bandwidth_card}
  {pool_card}
  {health_card}
</section>"#
    )
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.3} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn render_disposition_donut(metrics: &FlowMetrics) -> String {
    let blocked = metrics.blocked_requests;
    let direct = metrics.direct_bypassed_requests;
    let forwarded = metrics.forwarded_requests;
    let cached = metrics.cache_hits;
    let total = blocked
        .saturating_add(direct)
        .saturating_add(forwarded)
        .saturating_add(cached);

    // SVG donut geometry: radius = 52, circumference = 2 * pi * 52 = 326.7256
    let circumference = 326.7256_f64;
    let segments = [
        ("Blocked (0 B)", blocked, "#3fb950"),
        ("Direct Bypass", direct, "#58a6ff"),
        ("Upstream Proxy", forwarded, "#d29922"),
        ("HTTP Cache Hit", cached, "#39c5cf"),
    ];

    let mut circles = String::with_capacity(512);
    let mut offset = 0.0_f64;
    if total > 0 {
        for &(_, count, color) in &segments {
            if count == 0 {
                continue;
            }
            let frac = count as f64 / total as f64;
            let len = frac * circumference;
            circles.push_str(&format!(
                r#"<circle cx="64" cy="64" r="52" fill="none" stroke="{color}" stroke-width="18" stroke-dasharray="{len:.2} {circumference:.2}" stroke-dashoffset="-{offset:.2}" transform="rotate(-90 64 64)"/>"#
            ));
            offset += len;
        }
    }

    let mut legend = String::with_capacity(512);
    for &(label, count, color) in &segments {
        let pct = if total > 0 {
            (count as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        legend.push_str(&format!(
            r#"<div class="legend-row">
  <span class="legend-left"><span class="dot" style="background:{color};"></span>{label}</span>
  <span><strong>{count}</strong> <span class="muted">({pct:.1}%)</span></span>
</div>"#
        ));
    }

    format!(
        r##"<div class="card">
  <div class="chart-title">
    <span>🍩 Request Disposition Breakdown</span>
    <span class="muted">{total} events</span>
  </div>
  <div class="donut-wrap">
    <svg width="128" height="128" viewBox="0 0 128 128" role="img" aria-label="Request disposition donut chart">
      <circle cx="64" cy="64" r="52" fill="none" stroke="#21262d" stroke-width="18"/>
      {circles}
      <text x="64" y="61" text-anchor="middle" fill="#f0f6fc" font-size="16" font-weight="700">{total}</text>
      <text x="64" y="76" text-anchor="middle" fill="#8b949e" font-size="10">requests</text>
    </svg>
    <div class="legend">{legend}</div>
  </div>
</div>"##
    )
}

fn render_bandwidth_chart(metrics: &FlowMetrics) -> String {
    let saved = metrics.bytes_saved;
    let cache_saved = metrics.cache_bytes_saved;
    let upstream_tx = metrics.bytes_tx;
    let upstream_rx = metrics.bytes_rx;
    let upstream_total = metrics
        .upstream_bytes_used
        .max(upstream_tx.saturating_add(upstream_rx));
    let max_val = saved.max(upstream_total).max(cache_saved).max(1);

    let rows = [
        ("Bytes Saved (Blocked + Optimized)", saved, "#3fb950"),
        ("Paid Upstream Used (Total)", upstream_total, "#58a6ff"),
        ("Upstream TX (Sent)", upstream_tx, "#a371f7"),
        ("Upstream RX (Received)", upstream_rx, "#d29922"),
        ("HTTP Cache Saved", cache_saved, "#39c5cf"),
    ];

    let mut bars = String::with_capacity(768);
    for &(label, val, color) in &rows {
        let pct = ((val as f64 / max_val as f64) * 100.0).clamp(0.0, 100.0);
        let formatted = format_bytes(val);
        bars.push_str(&format!(
            r#"<div>
  <div class="bar-item-head">
    <span>{label}</span>
    <strong>{formatted}</strong>
  </div>
  <div class="bar-track">
    <div class="bar-seg" style="width:{pct:.1}%;background:{color};"></div>
  </div>
</div>"#
        ));
    }

    format!(
        r#"<div class="card">
  <div class="chart-title">
    <span>📶 Bandwidth Saved vs. Upstream Transfer</span>
    <span class="muted">Cache: {} hits / {} misses</span>
  </div>
  <div class="bar-list">{bars}</div>
</div>"#,
        metrics.cache_hits, metrics.cache_misses
    )
}

fn render_pool_traffic_chart(metrics: &FlowMetrics) -> String {
    if metrics.pool_stats.is_empty() {
        return r#"<div class="card">
  <div class="chart-title">
    <span>🔀 Traffic by Route / Proxy Pool</span>
    <span class="muted">0 pools active</span>
  </div>
  <div class="muted" style="font-size:13px;padding:24px 0;text-align:center;">
    No metered tunnel traffic recorded yet. Once requests flow through <code>direct</code>, <code>residential</code>, or other pools, per-pool TX/RX usage appears here.
  </div>
</div>"#
            .to_string();
    }

    let mut entries: Vec<(&String, u64, u64, u64)> = metrics
        .pool_stats
        .iter()
        .map(|(name, st)| (name, st.requests, st.bytes_tx, st.bytes_rx))
        .collect();
    entries.sort_by(|a, b| {
        let total_a = a.2.saturating_add(a.3);
        let total_b = b.2.saturating_add(b.3);
        total_b.cmp(&total_a).then_with(|| b.1.cmp(&a.1))
    });

    let max_bytes = entries
        .iter()
        .map(|e| e.2.saturating_add(e.3))
        .max()
        .unwrap_or(1)
        .max(1);

    let mut bars = String::with_capacity(entries.len() * 180);
    for (pool_name, reqs, tx, rx) in entries.iter().take(6) {
        let total = tx.saturating_add(*rx);
        let tx_pct = ((*tx as f64 / max_bytes as f64) * 100.0).clamp(0.0, 100.0);
        let rx_pct = ((*rx as f64 / max_bytes as f64) * 100.0).clamp(0.0, 100.0);
        let color_rx = if pool_name.eq_ignore_ascii_case("direct") {
            "#3fb950"
        } else {
            "#58a6ff"
        };
        bars.push_str(&format!(
            r#"<div>
  <div class="bar-item-head">
    <span><strong>{}</strong> <span class="muted">({reqs} reqs)</span></span>
    <span>{} <span class="muted">(TX {} / RX {})</span></span>
  </div>
  <div class="bar-track">
    <div class="bar-seg" style="width:{tx_pct:.1}%;background:#a371f7;" title="TX"></div>
    <div class="bar-seg" style="width:{rx_pct:.1}%;background:{color_rx};" title="RX"></div>
  </div>
</div>"#,
            html_escape(pool_name),
            format_bytes(total),
            format_bytes(*tx),
            format_bytes(*rx),
        ));
    }

    format!(
        r#"<div class="card">
  <div class="chart-title">
    <span>🔀 Traffic by Route / Proxy Pool</span>
    <span class="muted">{} active</span>
  </div>
  <div class="bar-list">{bars}</div>
</div>"#,
        metrics.pool_stats.len()
    )
}

fn render_proxy_health_chart(proxies: &[UpstreamProxy], now_secs: u64) -> String {
    let mut healthy = 0_u64;
    let mut recovering = 0_u64;
    let mut failing = 0_u64;
    let mut quarantined = 0_u64;

    for p in proxies {
        if p.is_auth_quarantined(now_secs) {
            quarantined += 1;
        } else if p.failure_count == 0 {
            healthy += 1;
        } else if p.failure_count < 3 {
            recovering += 1;
        } else {
            failing += 1;
        }
    }

    let total = proxies.len() as u64;
    let pct = |n: u64| -> f64 {
        if total > 0 {
            (n as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    };

    let h_pct = pct(healthy);
    let r_pct = pct(recovering);
    let f_pct = pct(failing);
    let q_pct = pct(quarantined);

    format!(
        r#"<div class="card">
  <div class="chart-title">
    <span>🩺 Proxy Pool Health &amp; 407 Quarantine</span>
    <span class="muted">{total} registered</span>
  </div>
  <div class="bar-track" style="height:16px;margin-bottom:14px;">
    <div class="bar-seg" style="width:{h_pct:.1}%;background:#3fb950;" title="Healthy: {healthy}"></div>
    <div class="bar-seg" style="width:{r_pct:.1}%;background:#d29922;" title="Recovering: {recovering}"></div>
    <div class="bar-seg" style="width:{f_pct:.1}%;background:#f85149;" title="Failing: {failing}"></div>
    <div class="bar-seg" style="width:{q_pct:.1}%;background:#a371f7;" title="407 Quarantined: {quarantined}"></div>
  </div>
  <div class="legend">
    <div class="legend-row">
      <span class="legend-left"><span class="dot" style="background:#3fb950;"></span>Healthy</span>
      <strong>{healthy}</strong>
    </div>
    <div class="legend-row">
      <span class="legend-left"><span class="dot" style="background:#d29922;"></span>Recovering (1–2 fails)</span>
      <strong>{recovering}</strong>
    </div>
    <div class="legend-row">
      <span class="legend-left"><span class="dot" style="background:#f85149;"></span>Failing (&ge;3 fails)</span>
      <strong>{failing}</strong>
    </div>
    <div class="legend-row">
      <span class="legend-left"><span class="dot" style="background:#a371f7;"></span>HTTP 407 Quarantined (TTL)</span>
      <strong>{quarantined}</strong>
    </div>
  </div>
</div>"#
    )
}
