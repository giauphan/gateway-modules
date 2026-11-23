"""
Self-contained HTML5 Web UI for Meta Business Suite / Facebook Creator Studio Adapter.
Clean Architecture Layer: Presentation Layer.
"""

import html
import json
from .client import build_masked_config_view, load_module_config


def render_module_ui_html(namespace: str = "/custom/facebook") -> str:
    raw_ns = (namespace or "/custom/facebook").rstrip("/") or "/custom/facebook"
    clean_ns = html.escape(raw_ns, quote=True)
    ns_js = json.dumps(raw_ns)
    cfg = load_module_config()
    masked = build_masked_config_view(cfg)

    base_url = html.escape(str(masked.get("base_url") or "https://business.facebook.com"), quote=True)
    enabled = masked.get("enabled", True)
    configured = masked.get("configured", False)
    user_id = html.escape(str(masked.get("user_id") or "not set"), quote=True)
    actor_id = html.escape(str(masked.get("actor_id") or "not set"), quote=True)
    target_id = html.escape(str(masked.get("target_id") or "not set"), quote=True)
    business_id = html.escape(str(masked.get("business_id") or "not set"), quote=True)
    masked_dtsg = html.escape(str(masked.get("masked_fb_dtsg") or "not set"), quote=True)
    masked_cookie = html.escape(str(masked.get("masked_cookie") or "not set"), quote=True)

    badge_text = "Configured" if configured else "Not Configured"
    badge_class = "badge-ok" if configured else "badge-warn"

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Meta Business Suite / Facebook Creator Studio Config</title>
<style>
  *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: system-ui, -apple-system, sans-serif; background: #0d0f14; color: #e2e8f0; padding: 32px 20px; line-height: 1.5; }}
  .container {{ max-width: 820px; margin: 0 auto; }}
  .header {{ display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 24px; gap: 12px; flex-wrap: wrap; }}
  h1 {{ font-size: 1.5rem; color: #f8fafc; margin-bottom: 6px; display: flex; align-items: center; gap: 8px; }}
  .sub {{ color: #94a3b8; font-size: 0.9rem; }}
  .nav-links {{ display: flex; gap: 8px; flex-wrap: wrap; }}
  .nav-link {{ padding: 8px 14px; border-radius: 8px; background: #1e2433; border: 1px solid #2e364f; color: #cbd5e1; text-decoration: none; font-size: 0.85rem; font-weight: 600; }}
  .card {{ background: #151922; border: 1px solid #232a3b; border-radius: 12px; padding: 24px; margin-bottom: 20px; }}
  .card-header {{ display: flex; justify-content: space-between; align-items: center; margin-bottom: 14px; gap: 10px; }}
  h2 {{ font-size: 1.1rem; color: #f1f5f9; }}
  .badge {{ padding: 4px 10px; border-radius: 999px; font-size: 0.75rem; font-weight: 600; }}
  .badge-ok {{ background: rgba(16, 185, 129, 0.15); color: #34d399; border: 1px solid rgba(16, 185, 129, 0.3); }}
  .badge-warn {{ background: rgba(245, 158, 11, 0.15); color: #fbbf24; border: 1px solid rgba(245, 158, 11, 0.3); }}
  .grid-2 {{ display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }}
  @media (max-width: 640px) {{ .grid-2 {{ grid-template-columns: 1fr; }} }}
  .field {{ margin-bottom: 14px; }}
  label {{ display: block; font-size: 0.85rem; font-weight: 600; color: #cbd5e1; margin-bottom: 6px; }}
  input, select, textarea {{ width: 100%; padding: 10px 12px; border-radius: 8px; background: #0d1017; border: 1px solid #2a3246; color: #f8fafc; font-size: 0.9rem; }}
  .hint {{ font-size: 0.78rem; color: #64748b; margin-top: 4px; }}
  .btn {{ padding: 10px 18px; border-radius: 8px; border: none; background: #0866ff; color: #fff; font-weight: 600; cursor: pointer; font-size: 0.9rem; transition: background 0.15s; }}
  .btn:hover {{ background: #0756d6; }}
  .btn-secondary {{ background: #1e2433; border: 1px solid #2e364f; color: #e2e8f0; }}
  .btn-secondary:hover {{ background: #262e40; }}
  .msg {{ margin-top: 12px; padding: 10px 14px; border-radius: 8px; font-size: 0.86rem; display: none; }}
  .msg-ok {{ display: block; background: rgba(16, 185, 129, 0.14); border: 1px solid rgba(16, 185, 129, 0.35); color: #6ee7b7; }}
  .msg-err {{ display: block; background: rgba(239, 68, 68, 0.14); border: 1px solid rgba(239, 68, 68, 0.35); color: #fca5a5; }}
  code {{ background: #0d1017; padding: 2px 6px; border-radius: 4px; color: #38bdf8; font-size: 0.84rem; }}
  .dropzone {{ border: 2px dashed #2a3246; border-radius: 10px; padding: 24px; text-align: center; cursor: pointer; transition: border-color 0.15s; }}
  .dropzone:hover {{ border-color: #0866ff; }}
</style>
</head>
<body>
<div class="container">
  <div class="header">
    <div>
      <h1>Meta Business Suite / Creator Studio</h1>
      <p class="sub">Namespace: <code>{clean_ns}</code> · Auto-extracts from <code>download-facebook.har</code></p>
    </div>
    <div class="nav-links">
      <a class="nav-link" href="/dashboard#provider-settings">← Provider Settings</a>
      <a class="nav-link" href="/dashboard#modules-section">Installed Modules</a>
    </div>
  </div>

  <div class="card">
    <div class="card-header">
      <h2>1. Import HAR Capture (download-facebook.har)</h2>
      <code>POST {clean_ns}/v1/config/har</code>
    </div>
    <div class="dropzone" id="har-dropzone" onclick="document.getElementById('har-file').click()">
      <p style="margin-bottom: 8px; font-weight: 500;">Click or drop <code>download-facebook.har</code> here</p>
      <p class="hint">Extracts <code>user_id</code>, <code>actor_id</code>, <code>target_id</code>, <code>__bid</code>, <code>fb_dtsg</code>, and Comet parameters automatically.</p>
      <input type="file" id="har-file" accept=".har,.json" style="display: none">
    </div>
    <div id="har-status" class="msg"></div>
  </div>

  <div class="card">
    <div class="card-header">
      <h2>2. Meta Business Suite Credentials &amp; Parameters</h2>
      <span class="badge {badge_class}" id="cfg-badge">{badge_text}</span>
    </div>
    <form id="fb-cfg-form">
      <div class="grid-2">
        <div class="field">
          <label for="base_url">Base URL</label>
          <input type="text" id="base_url" name="base_url" value="{base_url}">
        </div>
        <div class="field">
          <label for="enabled">Status</label>
          <select id="enabled" name="enabled">
            <option value="true" {'selected' if enabled else ''}>Enabled</option>
            <option value="false" {'' if enabled else 'selected'}>Disabled</option>
          </select>
        </div>
      </div>
      <div class="grid-2">
        <div class="field">
          <label for="user_id">Facebook User ID (__user)</label>
          <input type="text" id="user_id" name="user_id" placeholder="{user_id}">
          <p class="hint">Current: <code>{user_id}</code></p>
        </div>
        <div class="field">
          <label for="actor_id">Actor ID (av)</label>
          <input type="text" id="actor_id" name="actor_id" placeholder="{actor_id}">
          <p class="hint">Current: <code>{actor_id}</code></p>
        </div>
      </div>
      <div class="grid-2">
        <div class="field">
          <label for="target_id">Default Target Page ID</label>
          <input type="text" id="target_id" name="target_id" placeholder="{target_id}">
          <p class="hint">Current: <code>{target_id}</code></p>
        </div>
        <div class="field">
          <label for="business_id">Business ID (__bid)</label>
          <input type="text" id="business_id" name="business_id" placeholder="{business_id}">
          <p class="hint">Current: <code>{business_id}</code></p>
        </div>
      </div>
      <div class="field">
        <label for="fb_dtsg">Anti-CSRF Token (fb_dtsg)</label>
        <input type="password" id="fb_dtsg" name="fb_dtsg" placeholder="Leave empty to keep existing token">
        <p class="hint">Current: <code>{masked_dtsg}</code></p>
      </div>
      <div class="field">
        <label for="cookie">Session Cookie</label>
        <textarea id="cookie" name="cookie" rows="2" placeholder="Leave empty to keep existing cookie"></textarea>
        <p class="hint">Current: <code>{masked_cookie}</code></p>
      </div>
      <div style="display: flex; gap: 10px; margin-top: 16px;">
        <button type="submit" class="btn">Save Configuration</button>
        <button type="button" class="btn btn-secondary" onclick="checkStatus()">Refresh Status</button>
      </div>
      <div id="cfg-status" class="msg"></div>
    </form>
  </div>

  <div class="card">
    <div class="card-header">
      <h2>3. Active Scopes &amp; Capabilities</h2>
      <code>GET {clean_ns}/v1/scopes</code>
    </div>
    <p class="hint" style="margin-bottom: 12px;">Discovers connected Facebook Pages and Creator Studio publishing rights.</p>
    <div id="scopes-box" style="background: #0d1017; padding: 12px; border-radius: 8px; font-family: monospace; font-size: 0.85rem; color: #94a3b8;">
      Click "Check Scopes" to inspect live connected identities.
    </div>
    <div style="margin-top: 12px;">
      <button type="button" class="btn btn-secondary" onclick="fetchScopes()">Check Scopes</button>
    </div>
  </div>
</div>

<script>
  const NS = {ns_js};

  // HAR File Drag and Drop / Selection
  async function handleHarFile(file) {{
    if (!file) return;
    const statusEl = document.getElementById('har-status');
    statusEl.className = 'msg';
    statusEl.textContent = 'Parsing ' + file.name + '...';
    statusEl.style.display = 'block';

    try {{
      const text = await file.text();
      const res = await fetch(NS + '/v1/config/har', {{
        method: 'POST',
        headers: {{ 'Content-Type': 'application/json' }},
        body: text
      }});
      const data = await res.json();
      if (res.ok && data.status === 'ok') {{
        statusEl.className = 'msg msg-ok';
        statusEl.textContent = 'HAR credentials successfully extracted and saved!';
        setTimeout(() => location.reload(), 1200);
      }} else {{
        statusEl.className = 'msg msg-err';
        statusEl.textContent = data.error ? data.error.message : 'Failed to import HAR';
      }}
    }} catch (err) {{
      statusEl.className = 'msg msg-err';
      statusEl.textContent = 'Error reading file: ' + err.message;
    }}
  }}

  const dropzoneEl = document.getElementById('har-dropzone');
  if (dropzoneEl) {{
    dropzoneEl.addEventListener('dragover', (e) => {{
      e.preventDefault();
      e.stopPropagation();
      dropzoneEl.style.borderColor = '#0866ff';
    }});
    dropzoneEl.addEventListener('dragleave', (e) => {{
      e.preventDefault();
      e.stopPropagation();
      dropzoneEl.style.borderColor = '';
    }});
    dropzoneEl.addEventListener('drop', (e) => {{
      e.preventDefault();
      e.stopPropagation();
      dropzoneEl.style.borderColor = '';
      if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0]) {{
        handleHarFile(e.dataTransfer.files[0]);
      }}
    }});
  }}

  document.getElementById('har-file').addEventListener('change', (e) => {{
    if (e.target.files && e.target.files[0]) {{
      handleHarFile(e.target.files[0]);
    }}
  }});

  // Configuration Save
  document.getElementById('fb-cfg-form').addEventListener('submit', async (e) => {{
    e.preventDefault();
    const statusEl = document.getElementById('cfg-status');
    const form = e.target;
    const payload = {{
      base_url: form.base_url.value,
      enabled: form.enabled.value === 'true',
      user_id: form.user_id.value || undefined,
      actor_id: form.actor_id.value || undefined,
      target_id: form.target_id.value || undefined,
      business_id: form.business_id.value || undefined,
      fb_dtsg: form.fb_dtsg.value || undefined,
      cookie: form.cookie.value || undefined,
    }};

    try {{
      const res = await fetch(NS + '/v1/config', {{
        method: 'POST',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify(payload)
      }});
      const data = await res.json();
      if (res.ok) {{
        statusEl.className = 'msg msg-ok';
        statusEl.textContent = 'Configuration updated successfully!';
        setTimeout(() => location.reload(), 1000);
      }} else {{
        statusEl.className = 'msg msg-err';
        statusEl.textContent = data.error ? data.error.message : 'Save failed';
      }}
    }} catch (err) {{
      statusEl.className = 'msg msg-err';
      statusEl.textContent = 'Network error: ' + err.message;
    }}
  }});

  async function checkStatus() {{
    location.reload();
  }}

  async function fetchScopes() {{
    const box = document.getElementById('scopes-box');
    box.textContent = 'Fetching scopes...';
    try {{
      const res = await fetch(NS + '/v1/scopes');
      const data = await res.json();
      box.textContent = JSON.stringify(data, null, 2);
    }} catch (err) {{
      box.textContent = 'Error: ' + err.message;
    }}
  }}
</script>
</body>
</html>"""
