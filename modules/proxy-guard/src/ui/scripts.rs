//! Client-side interactive JavaScript for the Proxy Guard dashboard.
//!
//! Clean Architecture: Presentation / Interface Adapters Layer.

pub const DASHBOARD_JS: &str = r#"
function getApiEndpoint(subpath) {
  const p = window.location.pathname;
  if (p.includes('/custom/proxy-guard')) {
    return '/custom/proxy-guard' + subpath;
  }
  if (p.includes('/modules/proxy-guard')) {
    return '/modules/proxy-guard' + subpath;
  }
  return subpath;
}

function showToast(msg, isError) {
  const el = document.getElementById('toast');
  if (!el) return;
  el.textContent = msg;
  el.className = 'show ' + (isError ? 'err' : 'ok');
  clearTimeout(window.__toastTimer);
  window.__toastTimer = setTimeout(() => { el.className = ''; }, 3200);
}

function toggleActionMenu(ev) {
  ev.stopPropagation();
  const menu = document.getElementById('actionMenu');
  if (menu) menu.classList.toggle('open');
}

document.addEventListener('click', () => {
  const menu = document.getElementById('actionMenu');
  if (menu) menu.classList.remove('open');
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    const menu = document.getElementById('actionMenu');
    if (menu) menu.classList.remove('open');
  }
});

async function postJson(subpath, payload) {
  const res = await fetch(getApiEndpoint(subpath), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload || {})
  });
  return await res.json();
}

async function setStrategy(strategy) {
  try {
    const data = await postJson('/proxies/strategy', { strategy });
    if (data.status === 'ok') {
      showToast('Strategy switched to ' + strategy, false);
      setTimeout(() => location.reload(), 400);
    } else {
      showToast('Strategy error: ' + (data.error || JSON.stringify(data)), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function setDefaultRoute(route) {
  try {
    const data = await postJson('/rules', { default_route: route });
    if (data.status === 'ok') {
      showToast('Default route set to ' + route, false);
      setTimeout(() => location.reload(), 400);
    } else {
      showToast('Route error: ' + (data.error || JSON.stringify(data)), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function toggleCategory(field, enabled) {
  try {
    const payload = {};
    payload[field] = enabled;
    const data = await postJson('/rules', payload);
    if (data.status === 'ok') {
      showToast('Updated ' + field + ' = ' + enabled, false);
    } else {
      showToast('Failed to update rule: ' + (data.error || JSON.stringify(data)), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function submitQuickRule() {
  const domain = document.getElementById('ruleDomain').value.trim();
  const mode = document.getElementById('ruleAction').value;
  const pool = document.getElementById('rulePool').value.trim();
  if (!domain) { showToast('Enter a domain (e.g. services.sheerid.com)', true); return; }

  const payload = {};
  if (mode === 'block') {
    payload.add_blocked_domains = [domain];
  } else if (mode === 'direct') {
    payload.add_direct_bypass_domains = [domain];
  } else if (mode === 'whitelist') {
    payload.add_whitelist_domains = [domain];
  } else if (mode === 'pool') {
    if (!pool) { showToast('Enter a target pool name (e.g. residential)', true); return; }
    payload.add_pool_routes = [{ domain, pool }];
  }

  try {
    const data = await postJson('/rules', payload);
    if (data.status === 'ok') {
      showToast('Rule saved for ' + domain, false);
      setTimeout(() => location.reload(), 450);
    } else {
      showToast('Rule error: ' + (data.error || JSON.stringify(data)), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function resetMetrics() {
  if (!confirm('Reset all cumulative bandwidth & request metrics to zero?')) return;
  try {
    const data = await postJson('/metrics/reset', {});
    if (data.status === 'ok') {
      showToast('Metrics reset to zero', false);
      setTimeout(() => location.reload(), 400);
    } else {
      showToast('Reset failed: ' + (data.error || JSON.stringify(data)), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function unquarantineProxy(id) {
  if (!id) return;
  try {
    const data = await postJson('/proxies/report', { id, success: true });
    if (data.status === 'ok') {
      showToast('Restored proxy health: ' + id, false);
      setTimeout(() => location.reload(), 400);
    } else {
      showToast('Restore failed: ' + (data.error || JSON.stringify(data)), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function removeProxy(id) {
  if (!id || !confirm('Remove proxy ' + id + ' from pool?')) return;
  try {
    const data = await postJson('/proxies/remove', { id });
    if (data.status === 'ok') {
      location.reload();
    } else {
      showToast('Failed to remove proxy: ' + (data.error || 'Unknown error'), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function clearLogs() {
  if (!confirm('Clear recent intercepted request logs?')) return;
  try {
    const data = await postJson('/logs/clear', {});
    if (data.status === 'ok') {
      location.reload();
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

async function exportMetricsJson() {
  try {
    const res = await fetch(getApiEndpoint('/metrics'));
    const data = await res.json();
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'proxy-guard-metrics.json';
    a.click();
    URL.revokeObjectURL(url);
    showToast('Exported proxy-guard-metrics.json', false);
  } catch (err) {
    showToast('Export failed: ' + err, true);
  }
}

async function submitMultiProxy() {
  const raw = document.getElementById('proxyLines').value.trim();
  if (!raw) { showToast('Please enter at least one proxy URL', true); return; }
  const lines = raw.split('\n').map(l => l.trim()).filter(l => l.length > 0);
  let country = document.getElementById('countryCode').value.trim() || undefined;
  const appendCc = document.getElementById('appendCc').checked;
  if (country && appendCc && !country.toLowerCase().startsWith('-cc-') && !country.toLowerCase().startsWith('_cc_')) {
    country = '-cc-' + country.toLowerCase();
  }
  const tag = document.getElementById('proxyTag').value.trim() || undefined;
  const pool_name = document.getElementById('proxyPoolName').value.trim() || undefined;
  const tags = tag ? [tag] : undefined;

  const proxies = lines.map(url => ({ url, country, tags, pool_name }));
  try {
    const data = await postJson('/proxies/add', { proxies });
    if (data.status === 'ok') {
      showToast('Added ' + data.added_count + ' proxies to pool!', false);
      setTimeout(() => location.reload(), 500);
    } else {
      showToast('Error adding proxies: ' + JSON.stringify(data), true);
    }
  } catch (err) {
    showToast('Request failed: ' + err, true);
  }
}

function filterLogs() {
  const q = (document.getElementById('logSearch')?.value || '').toLowerCase();
  const rows = document.querySelectorAll('#logsTable tbody tr[data-log-row]');
  rows.forEach((tr) => {
    tr.style.display = tr.textContent.toLowerCase().includes(q) ? '' : 'none';
  });
}

function toggleRulePoolInput() {
  const mode = document.getElementById('ruleAction')?.value;
  const poolInput = document.getElementById('rulePool');
  if (poolInput) {
    poolInput.style.display = mode === 'pool' ? 'inline-block' : 'none';
  }
}

function initAutoRefresh() {
  const enabled = localStorage.getItem('pg_auto_refresh') === '1';
  const indicator = document.getElementById('autoRefreshState');
  if (indicator) indicator.textContent = enabled ? 'ON (10s)' : 'OFF';
  if (enabled) {
    window.__autoRefreshTimer = setInterval(() => {
      const activeTag = document.activeElement?.tagName;
      if (activeTag !== 'INPUT' && activeTag !== 'TEXTAREA' && activeTag !== 'SELECT') {
        location.reload();
      }
    }, 10000);
  }
}

function toggleAutoRefresh() {
  const next = localStorage.getItem('pg_auto_refresh') !== '1';
  localStorage.setItem('pg_auto_refresh', next ? '1' : '0');
  showToast('Auto-refresh ' + (next ? 'enabled (10s)' : 'disabled'), false);
  setTimeout(() => location.reload(), 300);
}

initAutoRefresh();
"#;
