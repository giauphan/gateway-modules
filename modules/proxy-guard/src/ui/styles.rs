//! Embedded CSS styles for the Proxy Guard interactive dashboard.
//!
//! Clean Architecture: Presentation / Interface Adapters Layer.

pub const DASHBOARD_CSS: &str = r#"
:root {
  --bg: #0d1117;
  --surface: #161b22;
  --surface-2: #21262d;
  --border: #30363d;
  --text: #c9d1d9;
  --muted: #8b949e;
  --title: #f0f6fc;
  --accent: #58a6ff;
  --green: #3fb950;
  --warn: #d29922;
  --err: #f85149;
  --purple: #a371f7;
  --cyan: #39c5cf;
}
* { box-sizing: border-box; }
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  background: var(--bg);
  color: var(--text);
  margin: 0;
  padding: 0 20px 48px;
}
.container { max-width: 1220px; margin: 0 auto; }
h1, h2, h3 { color: var(--title); margin-top: 0; }
.muted { color: var(--muted); }

/* Top Sticky Action Bar & Navigation Menu */
.topbar {
  position: sticky;
  top: 0;
  z-index: 40;
  background: rgba(13, 17, 23, 0.94);
  backdrop-filter: blur(8px);
  border-bottom: 1px solid var(--border);
  margin: 0 -20px 24px;
  padding: 12px 20px;
}
.topbar-inner {
  max-width: 1220px;
  margin: 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}
.brand { display: flex; align-items: center; gap: 10px; }
.brand h1 { font-size: 19px; margin: 0; }
.ver-pill {
  font-size: 11px;
  font-family: monospace;
  color: var(--accent);
  background: rgba(88, 166, 255, 0.12);
  border: 1px solid rgba(88, 166, 255, 0.3);
  border-radius: 999px;
  padding: 2px 8px;
}
.nav-links { display: flex; gap: 6px; flex-wrap: wrap; }
.nav-link {
  color: var(--text);
  text-decoration: none;
  font-size: 13px;
  padding: 6px 10px;
  border-radius: 6px;
  border: 1px solid transparent;
}
.nav-link:hover { background: var(--surface-2); border-color: var(--border); color: var(--title); }
.toolbar-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }

/* Action Dropdown Menu */
.action-menu { position: relative; display: inline-block; }
.action-menu-panel {
  display: none;
  position: absolute;
  right: 0;
  top: calc(100% + 6px);
  min-width: 250px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.55);
  padding: 6px;
  z-index: 60;
}
.action-menu.open .action-menu-panel { display: block; }
.menu-section {
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--muted);
  padding: 6px 10px 4px;
}
.menu-divider { height: 1px; background: var(--border); margin: 6px 0; }
.menu-item {
  width: 100%;
  background: transparent;
  border: 0;
  color: var(--text);
  text-align: left;
  padding: 7px 10px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.menu-item:hover { background: var(--surface-2); color: var(--title); }
.menu-item.danger { color: var(--err); }
.menu-item.danger:hover { background: rgba(248, 81, 73, 0.14); }
.menu-check { color: var(--green); font-weight: 700; }

/* Cards & Grids */
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(185px, 1fr));
  gap: 14px;
  margin-bottom: 20px;
}
.charts-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
  gap: 16px;
  margin-bottom: 22px;
}
.card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 16px;
}
.card-table { padding: 0; overflow-x: auto; margin-bottom: 22px; }
.card-header {
  padding: 14px 18px;
  border-bottom: 1px solid var(--border);
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}
.metric { font-size: 24px; font-weight: 700; color: var(--accent); margin-top: 4px; }
.card-green .metric { color: var(--green); }
.card-sub { font-size: 12px; color: var(--muted); margin-top: 3px; }

/* Charts */
.chart-title {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
  font-size: 14px;
  font-weight: 600;
  color: var(--title);
}
.donut-wrap { display: flex; align-items: center; gap: 18px; flex-wrap: wrap; }
.legend { display: flex; flex-direction: column; gap: 7px; flex: 1; min-width: 160px; }
.legend-row { display: flex; align-items: center; justify-content: space-between; font-size: 13px; gap: 8px; }
.legend-left { display: flex; align-items: center; gap: 8px; }
.dot { width: 10px; height: 10px; border-radius: 50%; display: inline-block; flex-shrink: 0; }
.bar-list { display: flex; flex-direction: column; gap: 10px; }
.bar-item-head { display: flex; justify-content: space-between; font-size: 12px; margin-bottom: 4px; gap: 8px; }
.bar-track { width: 100%; height: 9px; background: var(--surface-2); border-radius: 6px; overflow: hidden; display: flex; }
.bar-seg { height: 100%; }

/* Tables */
table { width: 100%; border-collapse: collapse; }
th, td { text-align: left; padding: 10px 12px; border-bottom: 1px solid var(--border); font-size: 13px; }
th { background: var(--surface-2); color: var(--muted); font-weight: 600; }
.badge { padding: 3px 8px; border-radius: 12px; font-size: 11px; font-weight: 600; text-transform: uppercase; white-space: nowrap; }
.badge.ok { background: rgba(63, 185, 80, 0.15); color: var(--green); }
.badge.warn { background: rgba(210, 153, 34, 0.15); color: var(--warn); }
.badge.err { background: rgba(248, 81, 73, 0.15); color: var(--err); }
.badge.quarantine { background: rgba(163, 113, 247, 0.18); color: var(--purple); border: 1px solid rgba(163, 113, 247, 0.35); }
.badge.info { background: rgba(88, 166, 255, 0.15); color: var(--accent); }
.tag { background: var(--surface-2); border: 1px solid var(--border); padding: 2px 6px; border-radius: 4px; font-size: 11px; }
.proto { font-weight: 600; color: #79c0ff; }
.success-count { color: var(--green); font-weight: 600; }
.fail-count { color: var(--err); font-weight: 600; }

/* Forms & Buttons */
.form-box { background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 18px; margin-bottom: 22px; }
textarea, select, input[type=text] {
  width: 100%;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--title);
  padding: 8px 10px;
  border-radius: 6px;
  font-family: monospace;
  font-size: 13px;
}
textarea { min-height: 92px; resize: vertical; margin-bottom: 12px; }
.row-flex { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
.toggle-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: var(--surface-2);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 6px 10px;
  font-size: 12px;
  cursor: pointer;
}
.btn {
  background: #238636;
  color: #fff;
  border: 1px solid rgba(240, 246, 252, 0.1);
  padding: 7px 14px;
  border-radius: 6px;
  cursor: pointer;
  font-weight: 600;
  font-size: 13px;
}
.btn:hover { background: #2ea043; }
.btn-secondary { background: var(--surface-2); color: var(--text); border: 1px solid var(--border); }
.btn-secondary:hover { background: #30363d; color: var(--title); }
.btn-sm { padding: 4px 9px; font-size: 12px; }
.btn-del { background: transparent; color: var(--err); border: 1px solid var(--err); }
.btn-del:hover { background: var(--err); color: #fff; }
.btn-restore { background: transparent; color: var(--purple); border: 1px solid var(--purple); }
.btn-restore:hover { background: var(--purple); color: #fff; }

/* Toast Notification */
#toast {
  position: fixed;
  right: 20px;
  bottom: 20px;
  z-index: 80;
  min-width: 240px;
  max-width: 420px;
  padding: 10px 14px;
  border-radius: 8px;
  background: var(--surface-2);
  border: 1px solid var(--border);
  color: var(--title);
  font-size: 13px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  display: none;
}
#toast.show { display: block; }
#toast.ok { border-color: var(--green); }
#toast.err { border-color: var(--err); }
"#;
