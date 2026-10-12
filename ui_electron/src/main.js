const { app, BrowserWindow, ipcMain, screen, Tray, Menu, nativeImage, shell } = require('electron');
const path = require('path');
const fs = require('fs');
const http = require('http');
const https = require('https');
const { execSync, spawn } = require('child_process');
const WebSocket = require('ws');

const envPath = path.join(__dirname, '..', '..', '.env');
if (fs.existsSync(envPath)) {
  const envContent = fs.readFileSync(envPath, 'utf8');
  envContent.split('\n').forEach(line => {
    const match = line.match(/^([^#=]+)=(.*)$/);
    if (match) process.env[match[1].trim()] = match[2].trim();
  });
}

// Control-plane base URL.
//
// The UI used to talk to the retired omp-server on :5656. It now talks to this repo's
// control plane, which already serves /status, /models, /main-agent and /ws — the exact
// shapes this file expects. Override with ELIA_CONTROL_URL to point elsewhere.
const API_BASE = (process.env.ELIA_CONTROL_URL || 'http://127.0.0.1:5680').replace(/\/+$/, '');
const WS_BASE = API_BASE.replace(/^http/, 'ws');

// The control plane runs with auth=on whenever .env defines ELIA_AUTH_TOKEN, and then
// every request without the bearer gets a 401. Shell out through `curlAuth` so the token
// rides along; the WebSocket carries it as a query param because browsers cannot set
// headers on a WS handshake.
const CONTROL_TOKEN = (process.env.ELIA_AUTH_TOKEN || '').trim();
const AUTH_HEADER = CONTROL_TOKEN ? ` -H "Authorization: Bearer ${CONTROL_TOKEN}"` : '';
const WS_AUTH_QUERY = CONTROL_TOKEN ? `?token=${encodeURIComponent(CONTROL_TOKEN)}` : '';

let mainWindow = null;
let setPositionOverlay = null;
let agentStatusInterval = null;
let tray = null;
let telegramBotProcess = null;
let discordBotProcess = null;
let configPath = path.join(__dirname, '..', 'config.json');
let config = {};
let ntfyStream = null;
let isProcessingCron = false;
let isProcessingMorning = false;
let morningPopup = null;
let cronPopup = null;
let subworkerPopup = null;
let welcomePopup = null;
let proxyPopup = null;
let welcomeShown = false;

const EliaAIRoot = path.join(__dirname, '..', '..');
const contextPath = path.join(EliaAIRoot, 'context');
const subworkersRoot = path.join(EliaAIRoot, 'subworkers');

// ── Main agent (default: elia) ───────────────────────────────
let _cachedMainAgent = 'elia';
function getMainAgentName() {
  try {
    const data = execSync(`curl -sf ${AUTH_HEADER} ${API_BASE}/main-agent`, { encoding: 'utf8', timeout: 3000 });
    const parsed = JSON.parse(data);
    if (parsed && typeof parsed.name === 'string' && parsed.name) {
      _cachedMainAgent = parsed.name;
      return parsed.name;
    }
  } catch (_) {}
  return _cachedMainAgent;
}

function loadContextFiles() {
  const context = { memory: '', tools: '', business: '' };
  try {
    context.memory = fs.readFileSync(path.join(contextPath, 'MEMORY.md'), 'utf8').substring(0, 500);
  } catch (e) {}
  try {
    context.tools = fs.readFileSync(path.join(contextPath, 'TOOLS.md'), 'utf8').substring(0, 500);
  } catch (e) {}
  try {
    context.business = fs.readFileSync(path.join(contextPath, 'business.md'), 'utf8').substring(0, 500);
  } catch (e) {}
  return context;
}

function loadConfig() {
  try {
    config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
    trayMenuState.statusLive = config.display?.statusLive ?? false;
    trayMenuState.voiceTrigger = config.display?.voiceTrigger ?? false;
  } catch (e) {
    console.error('Config load error:', e);
    config = { ntfy: { server: 'https://ntfy.sh', topic: 'test', token: '' }, display: {} };
  }
}

const WIN_W = 350;
const WIN_H = 310;
const positionPath = path.join(__dirname, '..', '.elia-position.json');
const SET_POSITION_FLAG = process.argv.includes('--set-position');

function getSavedPosition() {
  try {
    const data = JSON.parse(fs.readFileSync(positionPath, 'utf8'));
    if (typeof data.x === 'number' && typeof data.y === 'number') return { x: data.x, y: data.y };
  } catch (e) {}
  return null;
}

function savePosition(x, y) {
  try {
    fs.writeFileSync(positionPath, JSON.stringify({ x, y }) + '\n', 'utf8');
  } catch (e) {
    console.error('Save position:', e.message);
  }
}

function getDefaultBounds() {
  const saved = getSavedPosition();
  if (saved) return { x: saved.x, y: saved.y, width: WIN_W, height: WIN_H };
  const display = screen.getPrimaryDisplay();
  const { x: dx, y: dy, width: dw, height: dh } = display.bounds;
  return {
    x: dx + dw - WIN_W,
    y: dy + dh - WIN_H,
    width: WIN_W,
    height: WIN_H
  };
}

function createWindow() {
  loadConfig();
  const bounds = getDefaultBounds();

  mainWindow = new BrowserWindow({
    width: bounds.width,
    height: bounds.height,
    x: bounds.x,
    y: bounds.y,
    show: false,
    frame: false,
    transparent: true,
    alwaysOnTop: true,
    skipTaskbar: true,
    hasShadow: false,
    resizable: false,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    }
  });

  mainWindow.once('ready-to-show', () => {
    const b = getDefaultBounds();
    mainWindow.setBounds(b);
    mainWindow.show();
    setTimeout(() => mainWindow.setBounds(getDefaultBounds()), 50);
  });

  // Fix: Restore position when macOS menu bar or external forces move the window
  let _isRestoringPosition = false;
  mainWindow.on('moved', () => {
    if (_isRestoringPosition) return;
    _isRestoringPosition = true;
    // Restore saved position immediately (macOS menu bar can nudge the window)
    const saved = getSavedPosition();
    if (saved) {
      mainWindow.setPosition(saved.x, saved.y);
    }
    _isRestoringPosition = false;
  });

  mainWindow.loadFile(path.join(__dirname, 'index.html'));
  mainWindow.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: false });

  // Watch config changes
  fs.watch(configPath, () => {
    loadConfig();
    mainWindow?.webContents.send('config-updated', config);
    restartNtfyStream();
  });

  mainWindow.webContents.on('did-finish-load', () => {
    mainWindow.webContents.send('config-updated', config);
    startNtfyStream();
    startAgentStatusPolling();
    startSubworkerWebSocket();
    setTimeout(() => mainWindow?.setBounds(getDefaultBounds()), 150);
    mainWindow.setIgnoreMouseEvents(false, { forward: true });
  });
}

// ── Agent status (Docker subworker system) ──────────────────────────────
let _mainAgentRunning = false;
let _mainAgentStartedAt = 0;

function getAgentStatus() {
  try {
    const mainAgentName = getMainAgentName();
    const data = execSync(
      `curl -sf ${AUTH_HEADER} ${API_BASE}/status`,
      { encoding: 'utf8', timeout: 5000 }
    );
    const parsed = JSON.parse(data);
    const sw = (parsed.subworkers || []).find(s => s.name === mainAgentName);
    const running = sw ? (sw.running || false) : false;

    if (running && !_mainAgentRunning) {
      _mainAgentStartedAt = Date.now();
    } else if (!running) {
      _mainAgentStartedAt = 0;
    }
    _mainAgentRunning = running;

    if (!running) return { running: false };

    const elapsedSeconds = Math.floor((Date.now() - _mainAgentStartedAt) / 1000);
    let formatted = elapsedSeconds < 120 ? `${elapsedSeconds}s` : `${Math.floor(elapsedSeconds / 60)}m ${elapsedSeconds % 60}s`;
    if (elapsedSeconds >= 3600) formatted = `${Math.floor(elapsedSeconds / 3600)}h ${Math.floor((elapsedSeconds % 3600) / 60)}m`;

    return { running: true, elapsedSeconds, formatted };
  } catch (e) {
    return { running: false };
  }
}

function sendAgentStatus(status) {
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send('agent-status', status || getAgentStatus());
  }
}

function startAgentStatusPolling() {
  // Live elapsed counter only while the main agent runs — computed locally,
  // zero HTTP. State changes arrive via the subworker WebSocket.
  if (agentStatusInterval) clearInterval(agentStatusInterval);
  agentStatusInterval = setInterval(() => {
    if (_mainAgentRunning) sendAgentStatus();
  }, 1000);
}

// ── Subworker WebSocket (realtime — replaces HTTP polling) ──────────
let subworkerWS = null;
let wsReconnectDelay = 1000;
const wsSubworkerCache = new Map();   // name -> {enabled, running, model, variant}
let _descCache = null;

function getDescCache() {
  if (_descCache) return _descCache;
  _descCache = {};
  try {
    for (const sw of fs.readdirSync(subworkersRoot)) {
      const promptPath = path.join(subworkersRoot, sw, 'PROMPT.md');
      let description = '';
      try {
        description = fs.readFileSync(promptPath, 'utf8').split('\n')[0].replace(/^#\s*/, '').trim();
      } catch (e) {}
      _descCache[sw] = { description, path: path.join(subworkersRoot, sw) };
    }
  } catch (e) {}
  return _descCache;
}

function buildSubworkerList() {
  const mainAgentName = getMainAgentName();
  const desc = getDescCache();
  const results = [];
  for (const [name, st] of wsSubworkerCache) {
    results.push({
      name,
      label: name.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase()),
      description: desc[name]?.description || '',
      path: desc[name]?.path || path.join(subworkersRoot, name),
      enabled: st.enabled !== false,
      running: st.running || false,
      isMain: name === mainAgentName,
      schedule: st.schedule || null,
      model: st.model || null,
      variant: st.variant || null
    });
  }
  results.sort((a, b) => a.name.localeCompare(b.name));
  return results;
}

function pushSubworkerList() {
  if (subworkerPopup && !subworkerPopup.isDestroyed()) {
    subworkerPopup.webContents.send('subworkers-list', buildSubworkerList());
  }
}

function updateMainAgentFromCache() {
  const st = wsSubworkerCache.get(getMainAgentName());
  const running = st ? (st.running || false) : false;
  if (running && !_mainAgentRunning) _mainAgentStartedAt = Date.now();
  else if (!running) _mainAgentStartedAt = 0;
  _mainAgentRunning = running;
  sendAgentStatus();
}

function applyWsSnapshot(d) {
  for (const sw of (d.subworkers || [])) {
    const prev = wsSubworkerCache.get(sw.name) || {};
    wsSubworkerCache.set(sw.name, {
      ...prev,
      enabled: sw.enabled,
      running: sw.running,
      model: sw.model,
      variant: sw.variant,
      schedule: sw.schedule || prev.schedule || null
    });
  }
  pushSubworkerList();
  updateMainAgentFromCache();
}

function startSubworkerWebSocket() {
  const connect = () => {
    try {
      subworkerWS = new WebSocket(`${WS_BASE}/ws${WS_AUTH_QUERY}`);
    } catch (e) {
      console.error('WS unsupported:', e.message);
      return;
    }
    subworkerWS.on('open', () => { wsReconnectDelay = 1000; });
    subworkerWS.on('message', (raw) => {
      let d;
      try { d = JSON.parse(raw.toString()); } catch { return; }
      switch (d.event) {
        case 'initial_status':
        case 'status_update':
          applyWsSnapshot(d);
          break;
        case 'subworker_started': {
          const st = wsSubworkerCache.get(d.name);
          if (st) st.running = true;
          pushSubworkerList();
          updateMainAgentFromCache();
          break;
        }
        case 'subworker_completed':
        case 'subworker_error': {
          const st = wsSubworkerCache.get(d.name);
          if (st) st.running = false;
          pushSubworkerList();
          updateMainAgentFromCache();
          break;
        }
      }
    });
    subworkerWS.on('close', () => {
      setTimeout(connect, wsReconnectDelay);
      wsReconnectDelay = Math.min(wsReconnectDelay * 2, 15000);
    });
    subworkerWS.on('error', () => {});
  };
  connect();
}

// ── Subworker status ─────────────────────────────────────────
function parsePlistSchedule(plistPath) {
  try {
    if (!fs.existsSync(plistPath)) return null;
    const { execSync } = require('child_process');
    const json = execSync(`plutil -convert json -o - "${plistPath}" 2>/dev/null`, { encoding: 'utf8', timeout: 5000 }).trim();
    const plist = JSON.parse(json);

    // StartInterval (seconds-based, e.g. 1800 = every 30 min)
    if (plist.StartInterval) {
      const secs = plist.StartInterval;
      let label;
      if (secs >= 3600 && secs % 3600 === 0) label = `every ${secs / 3600}h`;
      else if (secs >= 60 && secs % 60 === 0) label = `every ${secs / 60}min`;
      else label = `every ${secs}s`;
      return {
        type: 'interval',
        description: label,
        intervalSeconds: secs,
        intervals: null,
        startHour: null,
        endHour: null,
        minutePattern: null
      };
    }

    // StartCalendarInterval (list of Hour+Minute dicts)
    const entries = plist.StartCalendarInterval;
    if (!entries || !Array.isArray(entries) || entries.length === 0) return null;

    const intervals = entries.map(e => ({ hour: e.Hour, minute: e.Minute, weekday: e.Weekday }));
    intervals.sort((a, b) => a.hour - b.hour || a.minute - b.minute);

    const hours = [...new Set(intervals.map(i => i.hour))].sort((a, b) => a - b);
    const minutes = [...new Set(intervals.map(i => i.minute))].sort((a, b) => a - b);
    const weekdays = [...new Set(intervals.map(i => i.weekday).filter(w => w !== undefined))].sort((a, b) => a - b);

    const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

    let description;
    if (intervals.length === 1 && weekdays.length === 1) {
      const e = intervals[0];
      description = `${DAY_NAMES[weekdays[0]]} ${String(e.hour).padStart(2, '0')}:${String(e.minute).padStart(2, '0')}`;
    } else if (intervals.length === 1) {
      const e = intervals[0];
      description = `${String(e.hour).padStart(2, '0')}:${String(e.minute).padStart(2, '0')} daily`;
    } else if (minutes.length === 1 && hours.length > 1) {
      const minStr = String(minutes[0]).padStart(2, '0');
      description = `${String(hours[0]).padStart(2, '0')}h→${String(hours[hours.length - 1]).padStart(2, '0')}h hourly at :${minStr}`;
    } else if (minutes.length === 2 && minutes[0] === 0 && minutes[1] === 30 && hours.length > 1) {
      description = `${String(hours[0]).padStart(2, '0')}h→${String(hours[hours.length - 1]).padStart(2, '0')}h every 30min`;
    } else {
      description = intervals.map(e => `${String(e.hour).padStart(2, '0')}:${String(e.minute).padStart(2, '0')}`).join(', ');
    }

    return {
      type: 'calendar',
      description,
      intervalSeconds: null,
      intervals,
      startHour: hours[0],
      endHour: hours[hours.length - 1],
      minutePattern: minutes
    };
  } catch (e) {
    console.error('parsePlistSchedule error:', e.message);
    return null;
  }
}

function getSubworkerStatus() {
  const results = [];
  try {
    const http = require('http');
    const data = require('child_process').execSync(
      `curl -sf ${AUTH_HEADER} ${API_BASE}/status`,
      { encoding: 'utf8', timeout: 5000 }
    );
    const parsed = JSON.parse(data);
    const mainAgentName = getMainAgentName();
    for (const sw of (parsed.subworkers || [])) {
      const promptPath = path.join(subworkersRoot, sw.name, 'PROMPT.md');
      let description = '';
      try {
        const firstLine = fs.readFileSync(promptPath, 'utf8').split('\n')[0] || '';
        description = firstLine.replace(/^#\s*/, '').trim();
      } catch (e) {
        description = '';
      }
      results.push({
        name: sw.name,
        label: sw.name.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase()),
        description,
        path: path.join(subworkersRoot, sw.name),
        enabled: sw.enabled !== false,
        running: sw.running || false,
        isMain: sw.name === mainAgentName,
        schedule: sw.schedule || null,
        model: sw.model || null,
        variant: sw.variant || null
      });
    }
    results.sort((a, b) => a.name.localeCompare(b.name));
  } catch (e) {
    console.error('getSubworkerStatus error:', e.message);
  }
  return results;
}

// ── ntfy SSE Stream ──────────────────────────────────────────
function startNtfyStream() {
  if (!config.ntfy?.topic) return;
  restartNtfyStream();
}

function restartNtfyStream() {
  if (ntfyStream) {
    try { ntfyStream.destroy(); } catch (e) {}
    ntfyStream = null;
  }
  const { server, topic, token } = config.ntfy || {};
  if (!server || !topic) return;

  const url = `${server}/${topic}/json`;
  const parsed = new URL(url);
  const lib = parsed.protocol === 'https:' ? https : http;

  const headers = { 'Accept': 'application/x-ndjson' };
  if (token) headers['Authorization'] = `Bearer ${token}`;

  const req = lib.get(url, { headers }, (res) => {
    ntfyStream = res;
    let buf = '';
    res.on('data', chunk => {
      buf += chunk.toString();
      const lines = buf.split('\n');
      buf = lines.pop();
      lines.forEach(line => {
        line = line.trim();
        if (!line) return;
        try {
          const msg = JSON.parse(line);
          if (msg.event === 'message') {
            mainWindow?.webContents.send('ntfy-message', {
              id: msg.id,
              time: msg.time,
              title: msg.title || topic,
              message: msg.message || '',
              priority: msg.priority || 3,
              tags: msg.tags || []
            });
          }
        } catch (e) {}
      });
    });
    res.on('error', () => scheduleReconnect());
    res.on('close', () => scheduleReconnect());
  });

  req.on('error', () => scheduleReconnect());
}

let reconnectTimer = null;
function scheduleReconnect() {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    restartNtfyStream();
  }, 5000);
}

// ── IPC ───────────────────────────────────────────────────────
ipcMain.on('hide-window', () => mainWindow?.hide());
ipcMain.on('show-window', () => mainWindow?.show());
ipcMain.on('get-selected-model', (event) => {
  // This will be handled by the renderer process
  // We need to get the selected model from the renderer
  mainWindow?.webContents.executeJavaScript(`
    localStorage.getItem('selectedModel') || 'opencode/big-pickle'
  `).then(model => {
    event.reply('selected-model', model);
  });
});

ipcMain.on('set-selected-model', (event, model) => {
  mainWindow?.webContents.executeJavaScript(`
    localStorage.setItem('selectedModel', '${model}');
    selectModel('${model}');
  `);
  writeModelForCron(model);
});

// Persist selected model for cron/trigger (Elia choice = cron job model)
const opencodeModelPath = path.join(EliaAIRoot, '.opencode_model');
const MODEL_SHORT_MAP = {
  'opencode/x-preview-f-free': 'x-preview-f-free',
  'opencode/big-pickle': 'big-pickle',
  'opencode/deepseek-v4-flash-free': 'deepseek-v4-flash-free',
  'opencode/mimo-v2.5-free': 'mimo-v2.5-free',
  'opencode/nemotron-3-ultra-free': 'nemotron-3-ultra-free'
};
const SAFE_MODELS = ['x-preview-f-free', 'big-pickle', 'deepseek-v4-flash-free', 'mimo-v2.5-free', 'nemotron-3-ultra-free', 'minimax', 'nvidia', 'deepseek', 'mimo', 'nemotron'];
function writeModelForCron(model) {
  if (!model || typeof model !== 'string') return;
  const short = MODEL_SHORT_MAP[model] || model;
  const safe = SAFE_MODELS.includes(short) ? short : 'big-pickle';
  try {
    fs.writeFileSync(opencodeModelPath, safe + '\n', 'utf8');
  } catch (e) {
    console.error('writeModelForCron:', e.message);
  }
}
ipcMain.on('save-model-for-cron', (_event, model) => {
  writeModelForCron(model);
});

// OMO is always enabled. Ralph/ULW toggle uses .ralph_mode file.
const opencodeRalphPath = path.join(EliaAIRoot, '.ralph_mode');

function writeRalphMode(enabled) {
  try {
    if (enabled) {
      fs.writeFileSync(opencodeRalphPath, 'enabled\n', 'utf8');
    } else {
      if (fs.existsSync(opencodeRalphPath)) fs.unlinkSync(opencodeRalphPath);
    }
    console.log('Ralph mode:', enabled ? 'enabled (ULW disabled)' : 'disabled (ULW default)');
  } catch (e) {
    console.error('writeRalphMode:', e.message);
  }
}

ipcMain.on('omo-toggle', (_event, _enabled) => {
  console.log('OMO toggle ignored — always enabled');
});

ipcMain.on('ulw-toggle', (_event, enabled) => {
  writeRalphMode(!enabled);
});

const manageCronScript = path.join(EliaAIRoot, 'scripts/manage_cron.sh');

ipcMain.on('cron-toggle', (_event, { action }) => {
  const { exec } = require('child_process');
  if (action !== 'install' && action !== 'uninstall') {
    console.error('cron-toggle: unknown action', action);
    return;
  }
  const agentName = getMainAgentName();
  const endpoint = action === 'install' ? 'enable' : 'disable';
  const url = `${API_BASE}/${endpoint}/${encodeURIComponent(agentName)}`;
  exec(`curl -s${AUTH_HEADER} -X POST "${url}"`, (error, stdout) => {
    if (error) {
      console.error('cron-toggle error:', error.message);
    } else {
      console.log(`cron-toggle: ${action} ${agentName} ->`, stdout);
    }
  });
});

ipcMain.on('show-cron-popup', () => {
  showCronConfirmation();
});

// Subworker popup
ipcMain.on('show-subworker-popup', () => {
  showSubworkerPopupWindow();
});

let schedulePickerPopup = null;
ipcMain.on('open-schedule-picker', (event, agentName, currentSchedule) => {
  showSchedulePickerWindow(agentName, currentSchedule);
});

ipcMain.on('save-subworker-schedule', (event, { agentName, scheduleConfig }) => {
  try {
    const plistPath = path.join(subworkersRoot, 'plists', `com.elia.${agentName}.plist`);
    if (!fs.existsSync(plistPath)) {
      event.reply('schedule-saved', { success: false, error: 'Plist not found' });
      return;
    }

    const json = execSync(`plutil -convert json -o - "${plistPath}"`, { encoding: 'utf8' });
    const plist = JSON.parse(json);

    delete plist.StartInterval;
    delete plist.StartCalendarInterval;

    if (scheduleConfig.type === 'interval') {
      plist.StartInterval = scheduleConfig.seconds;
    } else if (scheduleConfig.type === 'calendar') {
      plist.StartCalendarInterval = scheduleConfig.entries;
    }

    const tmpJson = path.join('/tmp', `plist_${agentName}_${Date.now()}.json`);
    fs.writeFileSync(tmpJson, JSON.stringify(plist), 'utf8');
    execSync(`plutil -convert xml1 -o "${plistPath}" "${tmpJson}"`, { encoding: 'utf8' });
    fs.unlinkSync(tmpJson);

    try {
      execSync(`launchctl bootout gui/$(id -u) "${plistPath}" 2>/dev/null || true`, { encoding: 'utf8' });
    } catch (_) {}
    try {
      execSync(`launchctl bootstrap gui/$(id -u) "${plistPath}" 2>/dev/null || true`, { encoding: 'utf8' });
    } catch (_) {}

    if (subworkerPopup && !subworkerPopup.isDestroyed()) {
      subworkerPopup.webContents.send('subworkers-list', getSubworkerStatus());
    }

    event.reply('schedule-saved', { success: true, agentName });
  } catch (e) {
    console.error('save-subworker-schedule error:', e.message);
    event.reply('schedule-saved', { success: false, error: e.message });
  }
});

// Get subworker list
ipcMain.on('get-subworkers', (event) => {
  const workers = wsSubworkerCache.size > 0 ? buildSubworkerList() : getSubworkerStatus();
  event.reply('subworkers-list', workers);
});

ipcMain.on('set-main-agent', (event, name) => {
  const clean = typeof name === 'string' ? name.replace(/[^a-zA-Z0-9_-]/g, '') : '';
  try {
    const agentName = clean || 'elia';
    execSync(`curl -sf ${AUTH_HEADER} -X POST ${API_BASE}/main-agent -H "Content-Type: application/json" -d '{"name":"${agentName}"}'`, { encoding: 'utf8', timeout: 5000 });
    _cachedMainAgent = agentName;
    if (subworkerPopup && !subworkerPopup.isDestroyed()) {
      subworkerPopup.webContents.send('subworkers-list', getSubworkerStatus());
    }
    event.reply('main-agent-set', { success: true, name: agentName });
  } catch (e) {
    console.error('set-main-agent error:', e.message);
    event.reply('main-agent-set', { success: false, error: e.message });
  }
});

// Toggle subworker enabled state
ipcMain.on('toggle-subworker', (event, name) => {
  if (!name) return;
  const { execSync } = require('child_process');

  try {
    const current = JSON.parse(
      execSync(`curl -sf ${AUTH_HEADER} ${API_BASE}/status`, { encoding: 'utf8', timeout: 5000 })
    );
    const sw = (current.subworkers || []).find(s => s.name === name);
    const currentlyEnabled = sw ? sw.enabled !== false : false;
    const endpoint = currentlyEnabled ? 'disable' : 'enable';
    const result = JSON.parse(
      execSync(`curl -sf ${AUTH_HEADER} -X POST ${API_BASE}/${endpoint}/${encodeURIComponent(name)}`, {
        encoding: 'utf8', timeout: 5000
      })
    );
    event.reply('subworker-toggled', { name, enabled: result.enabled, running: false });
  } catch (e) {
    console.error(`toggle-subworker ${name}:`, e.message);
    event.reply('subworker-toggled', { name, enabled: false, running: false });
  }
});

// Run Morning Routine
ipcMain.on('run-morning-routine', () => {
  const { exec } = require('child_process');
  const triggerUrl = `${API_BASE}/trigger/elia`;
  exec(`curl -s${AUTH_HEADER} -X POST "${triggerUrl}"`, (error, stdout, stderr) => {
    if (error) {
      console.error('Morning routine error:', error.message);
    } else {
      console.log('Morning routine triggered:', stdout);
    }
  });
});

// Run Morning Speak (vocal briefing)
ipcMain.on('run-morning-speak', () => {
  runMorningSpeak();
});

// Close all popups
ipcMain.on('close-popup', () => {
  if (morningPopup && !morningPopup.isDestroyed()) {
    morningPopup.close();
  }
  if (cronPopup && !cronPopup.isDestroyed()) {
    cronPopup.close();
  }
  if (subworkerPopup && !subworkerPopup.isDestroyed()) {
    subworkerPopup.close();
  }
  if (welcomePopup && !welcomePopup.isDestroyed()) {
    welcomePopup.close();
  }
  if (schedulePickerPopup && !schedulePickerPopup.isDestroyed()) {
    schedulePickerPopup.close();
  }
});

// Run Manual Cron (from cron popup)
ipcMain.on('run-cron-manual', () => {
  const { exec } = require('child_process');
  const triggerUrl = `${API_BASE}/trigger/elia`;
  exec(`curl -s${AUTH_HEADER} -X POST "${triggerUrl}"`, (error, stdout, stderr) => {
    if (error) {
      console.error('Manual cron error:', error.message);
    } else {
      console.log('Manual cron triggered:', stdout);
    }
  });
});

// Open URL in browser
ipcMain.on('open-url', (_event, url) => {
  shell.openExternal(url);
});

// Open logs terminal
ipcMain.on('open-logs-terminal', () => {
  const { exec } = require('child_process');
  const fs = require('fs');
  const path = require('path');
  
  // Find latest opencode_interactive log file
  const logsDir = '/Users/vakandi/EliaAI/logs';
  let latestLog = null;
  let latestTime = 0;
  
  try {
    const files = fs.readdirSync(logsDir);
    for (const file of files) {
      if (file.startsWith('opencode_interactive_') && file.endsWith('.log')) {
        const filePath = path.join(logsDir, file);
        const stats = fs.statSync(filePath);
        if (stats.mtimeMs > latestTime) {
          latestTime = stats.mtimeMs;
          latestLog = filePath;
        }
      }
    }
  } catch (e) {}
  
  // Fallback to cron.log if no opencode log found
  const logFile = latestLog || '/Users/vakandi/EliaAI/logs/cron.log';
  // Show whole file + follow in realtime
  const cmd = 'tail -n 5000 -f ' + logFile;
  
  const script = [
    'tell application "Terminal" to activate',
    'tell application "Terminal" to do script ' + JSON.stringify(cmd),
    'delay 0.3',
    'tell application "Terminal" to set bounds of front window to {20, 50, 900, 900}'
  ].map(s => '-e ' + JSON.stringify(s)).join(' ');
  exec('osascript ' + script, (error) => {
    if (error) console.error('Logs terminal error:', error.message);
  });
});

ipcMain.on('open-subworker-logs', (_event, name) => {
  if (!name) return;
  const { exec } = require('child_process');
  // Try to open the LATEST per-run log file first, fallback to aggregate
  const runsDir = path.join(subworkersRoot, 'logs', 'runs', name.replace(/-/g, '_'));
  let logFile = path.join(subworkersRoot, 'logs', name.replace(/-/g, '_') + '.log');
  try {
    if (fs.existsSync(runsDir)) {
      const files = fs.readdirSync(runsDir)
        .filter(f => f.endsWith('.log'))
        .sort()
        .reverse();
      if (files.length > 0) {
        logFile = path.join(runsDir, files[0]);
      }
    }
  } catch (_) {}
  if (!fs.existsSync(logFile)) {
    const { dialog } = require('electron');
    dialog.showMessageBox({
      type: 'info',
      title: 'No logs',
      message: `No run logs yet for ${name}.`,
      detail: 'Trigger the subworker once (server) to generate the first log file.'
    });
    return;
  }
  const cmd = 'tail -n 5000 -f ' + logFile;
  const script = [
    'tell application "Terminal" to activate',
    'tell application "Terminal" to do script ' + JSON.stringify(cmd),
    'delay 0.3',
    'tell application "Terminal" to set bounds of front window to {20, 50, 900, 500}'
  ].map(s => '-e ' + JSON.stringify(s)).join(' ');
  exec('osascript ' + script, (error) => {
    if (error) console.error('open-subworker-logs error:', error.message);
  });
});

ipcMain.on('run-subworker-now', (_event, name, model) => {
  if (!name) return;
  const { exec } = require('child_process');
  const triggerUrl = `${API_BASE}/trigger/${encodeURIComponent(name)}`;
  const termCmd = `curl -s${AUTH_HEADER} -X POST "${triggerUrl}" | tee /tmp/subworker_run_${name}.log; echo "EXIT:$?"`;
  const script = [
    'tell application "Terminal" to activate',
    'tell application "Terminal" to do script ' + JSON.stringify(termCmd),
    'delay 0.3',
    'tell application "Terminal" to set bounds of front window to {20, 50, 900, 500}'
  ].map(s => '-e ' + JSON.stringify(s)).join(' ');
  exec('osascript ' + script, (error) => {
    if (error) { console.error('run-subworker-now error:', error.message); return; }
    try {
      const statusData = require('child_process').execSync(
        `curl -sf ${AUTH_HEADER} ${API_BASE}/status`, { encoding: 'utf8', timeout: 5000 }
      );
      const parsed = JSON.parse(statusData);
      const sw = (parsed.subworkers || []).find(s => s.name === name);
      const enabled = sw ? sw.enabled !== false : false;
      if (subworkerPopup && !subworkerPopup.isDestroyed()) {
        subworkerPopup.webContents.send('subworker-toggled', { name, enabled, running: true });
      }
    } catch (e) {
      if (subworkerPopup && !subworkerPopup.isDestroyed()) {
        subworkerPopup.webContents.send('subworker-toggled', { name, enabled: true, running: true });
      }
    }
  });
});

// ── Model selection persistence ──────────────────────────────
const modelSelectionsPath = path.join(__dirname, '..', 'model-selections.json');

ipcMain.handle('get-model-selections', () => {
  try {
    if (fs.existsSync(modelSelectionsPath)) {
      return JSON.parse(fs.readFileSync(modelSelectionsPath, 'utf-8'));
    }
  } catch (e) {
    console.error('get-model-selections error:', e.message);
  }
  return {};
});

ipcMain.on('save-model-selection', (_event, name, model) => {
  try {
    let data = {};
    if (fs.existsSync(modelSelectionsPath)) {
      data = JSON.parse(fs.readFileSync(modelSelectionsPath, 'utf-8'));
    }
    data[name] = model;
    fs.writeFileSync(modelSelectionsPath, JSON.stringify(data, null, 2));
  } catch (e) {
    console.error('save-model-selection error:', e.message);
  }
});

// ── Helpers for run log timestamp parsing ──────────────────────────
function parseLocalTimestamp(str) {
  const parts = str.match(/\d+/g);
  if (!parts || parts.length < 6) return null;
  return new Date(+parts[0], +parts[1] - 1, +parts[2], +parts[3], +parts[4], +parts[5]);
}

function timeAgo(date) {
  if (!date) return null;
  const diffMs = Date.now() - date.getTime();
  if (diffMs < 60000) return 'just now';
  if (diffMs < 3600000) return Math.floor(diffMs / 60000) + 'm ago';
  if (diffMs < 86400000) {
    const h = Math.floor(diffMs / 3600000);
    const m = Math.floor((diffMs % 3600000) / 60000);
    return h + 'h ' + m + 'm ago';
  }
  return Math.floor(diffMs / 86400000) + 'd ago';
}

function formatDuration(ms) {
  if (ms == null || ms <= 0) return null;
  const totalSec = Math.round(ms / 1000);
  if (totalSec < 60) return totalSec + 's';
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  if (m < 60) return m + 'm ' + s + 's';
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return h + 'h ' + rm + 'm';
}

// List per-run log files for a subworker (last 100, sorted newest first)
ipcMain.on('get-subworker-runs', (event, name) => {
  if (!name) return;
  const runsDir = path.join(subworkersRoot, 'logs', 'runs', name.replace(/-/g, '_'));
  const runs = [];
  try {
    if (fs.existsSync(runsDir)) {
      const files = fs.readdirSync(runsDir)
        .filter(f => f.endsWith('.log'))
        .sort()
        .reverse()
        .slice(0, 100);
      for (const file of files) {
        const filePath = path.join(runsDir, file);
        try {
          const stats = fs.statSync(filePath);
          const run = { filename: file, path: filePath, mtime: stats.mtimeMs, size: stats.size, status: 'unknown' };

          // Parse log content for timeAgo and duration
          try {
            const fd = fs.openSync(filePath, 'r');
            const headBuf = Buffer.alloc(300);
            const headBytes = fs.readSync(fd, headBuf, 0, 300, 0);
            const head = headBuf.toString('utf8', 0, headBytes);
            const startMatch = head.match(/^\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})\]/);
            if (startMatch) {
              const startDate = parseLocalTimestamp(startMatch[1]);
              run.timeAgo = timeAgo(startDate);

              // Read tail to find status marker
              // Legacy run logs end with: [timestamp] EOF_SUBWORKER_EXIT:<code>
              // This is a unique string the AI can never produce.
              const tailLen = Math.min(stats.size, 300);
              const tailBuf = Buffer.alloc(tailLen);
              const tailBytes = fs.readSync(fd, tailBuf, 0, tailLen, Math.max(0, stats.size - tailLen));
              const tail = tailBuf.toString('utf8', 0, tailBytes);
              const exitMatch = tail.match(/\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})\]\s+EOF_SUBWORKER_EXIT:(\d+)/);
              if (exitMatch) {
                const endDate = parseLocalTimestamp(exitMatch[1]);
                const exitCode = parseInt(exitMatch[2], 10);
                run.duration = formatDuration(endDate - startDate);
                if (exitCode !== 0) run.status = 'crashed';
                else run.status = 'success';
              } else {
                // No EOF marker — old log or pre-marker run. Try partial
                // duration from last [timestamp] line, status stays unknown.
                const allTs = [...tail.matchAll(/\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})\]/g)];
                if (allTs.length > 0) {
                  const lastDate = parseLocalTimestamp(allTs[allTs.length - 1][1]);
                  run.duration = formatDuration(lastDate - startDate);
                }
              }
            }
            fs.closeSync(fd);
          } catch (_) {}

          runs.push(run);
        } catch (_) {}
      }
    }
  } catch (_) {}
  event.reply('subworker-runs', { name, runs });
});

// Open a specific log file in Terminal
ipcMain.on('open-subworker-log-file', (_event, logFilePath) => {
  if (!logFilePath) return;
  const { exec } = require('child_process');
  const cmd = 'tail -n 5000 -f ' + logFilePath;
  const script = [
    'tell application "Terminal" to activate',
    'tell application "Terminal" to do script ' + JSON.stringify(cmd),
    'delay 0.3',
    'tell application "Terminal" to set bounds of front window to {20, 50, 900, 500}'
  ].map(s => '-e ' + JSON.stringify(s)).join(' ');
  exec('osascript ' + script, (error) => {
    if (error) console.error('open-subworker-log-file error:', error.message);
  });
});

ipcMain.on('get-toggle-states', (event) => {
  const ulwEnabled = !fs.existsSync(opencodeRalphPath);
  const cronSettings = getCurrentCronSettings();
  
  event.reply('toggle-states', { 
    omoEnabled: true, 
    ulwEnabled,
    cronEnabled: cronSettings.standardEnabled,
    cronInterval: cronSettings.interval,
    cronStartHour: cronSettings.startHour,
    cronEndHour: cronSettings.endHour
  });
});

ipcMain.on('get-config', (evt) => evt.reply('config-updated', config));
ipcMain.on('quit-app', () => app.quit());
ipcMain.on('set-ignore-mouse-events', (event, ignore, options) => {
  mainWindow?.setIgnoreMouseEvents(ignore, options);
});
ipcMain.on('execute-elia-command', () => {
  const { exec } = require('child_process');
  mainWindow?.webContents.send('recording-started');

  mainWindow.webContents.executeJavaScript('localStorage.getItem("selectedModel") || "minimax"')
    .then(model => {
      const rawModel = (model || 'opencode/big-pickle').replace(/^opencode\//, '');

      const proxyEnabled = fs.existsSync(proxyStatePath);

      let cmd;
      if (proxyEnabled) {
        const proxyConf = require('path').join(require('os').homedir(), '.proxychains.conf');
        try {
          const proxyContent = fs.readFileSync(proxyConf, 'utf8');
          const proxyLine = proxyContent.split('\n').find(l => l.trim() && !l.trim().startsWith('#') && l.includes('http '));
          if (proxyLine) {
            const parts = proxyLine.trim().split(/\s+/);
            if (parts.length >= 5) {
              const ip = parts[1];
              const port = parts[2];
              const user = parts[3];
              const pass = parts[4];
              const proxyUrl = `http://${user}:${pass}@${ip}:${port}`;
              cmd = `cd /Users/vakandi/EliaAI && env HTTP_PROXY="${proxyUrl}" HTTPS_PROXY="${proxyUrl}" http_proxy="${proxyUrl}" https_proxy="${proxyUrl}" ELIA_MODEL=${rawModel} /Users/vakandi/Documents/dictate.command`;
            }
          }
        } catch (e) {
          console.error('Proxy config read error:', e.message);
        }
      }

      if (!cmd) {
        cmd = `cd /Users/vakandi/EliaAI && ELIA_MODEL=${rawModel} /Users/vakandi/Documents/dictate.command`;
      }

      const script = [
        'tell application "Terminal" to activate',
        'tell application "Terminal" to do script ' + JSON.stringify(cmd),
        'delay 0.3',
        'tell application "Terminal" to set bounds of front window to {100, 50, 500, 900}'
      ].map(s => '-e ' + JSON.stringify(s)).join(' ');
      exec(`osascript ${script}`, (error, stdout, stderr) => {
        if (error) console.error('Execution error:', error.message);
      });
    })
    .catch(err => {
      console.error('get selectedModel:', err);
      mainWindow?.webContents.send('recording-stopped');
    });
});

ipcMain.on('execute-mini-orb', () => {
  const { exec } = require('child_process');
  const proxyEnabled = fs.existsSync(proxyStatePath);
  
  let cmd;
  if (proxyEnabled) {
    const proxyConf = require('path').join(require('os').homedir(), '.proxychains.conf');
    try {
      const proxyContent = fs.readFileSync(proxyConf, 'utf8');
      const proxyLine = proxyContent.split('\n').find(l => l.trim() && !l.trim().startsWith('#') && l.includes('http '));
      if (proxyLine) {
        const parts = proxyLine.trim().split(/\s+/);
        if (parts.length >= 5) {
          const ip = parts[1];
          const port = parts[2];
          const user = parts[3];
          const pass = parts[4];
          const proxyUrl = `http://${user}:${pass}@${ip}:${port}`;
          cmd = `env HTTP_PROXY="${proxyUrl}" HTTPS_PROXY="${proxyUrl}" http_proxy="${proxyUrl}" https_proxy="${proxyUrl}" /Users/vakandi/EliaAI/scripts/voice-command-only.sh`;
        }
      }
    } catch (e) {
      console.error('Proxy config read error:', e.message);
    }
  }
  
  if (!cmd) {
    cmd = '/Users/vakandi/EliaAI/scripts/voice-command-only.sh';
  }
  
  const script = [
    'tell application "Terminal" to activate',
    'delay 0.5',
    'tell application "Terminal" to do script ' + JSON.stringify(cmd),
    'delay 0.5',
    'tell application "Terminal" to set bounds of front window to {100, 50, 500, 900}'
  ].map(s => `-e ${JSON.stringify(s)}`).join(' ');
  exec(`osascript ${script}`, (error, stdout, stderr) => {
    if (error) console.error('Mini orb execution error:', error.message);
  });
});

ipcMain.on('stop-recording', () => {
  const { exec } = require('child_process');
  exec('osascript -e \'tell application "System Events" to keystroke "c" using command down\'', (error) => {
    if (error) console.error('Stop recording:', error.message);
  });
  mainWindow?.webContents.send('recording-stopped');
});

ipcMain.on('set-position-click', (_event, screenX, screenY) => {
  const x = Math.round(screenX - WIN_W / 2);
  const y = Math.round(screenY - WIN_H / 2);
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.setBounds({ x, y, width: WIN_W, height: WIN_H });
    mainWindow.show();
  }
  savePosition(x, y);
  if (setPositionOverlay && !setPositionOverlay.isDestroyed()) {
    setPositionOverlay.close();
    setPositionOverlay = null;
  }
});

// ── Tray ──────────────────────────────────────────────────────
let trayMenuState = {
  statusLive: false,
  voiceTrigger: false,
  selectedModel: 'minimax',
  proxyEnabled: false
};

// Proxy state file
const proxyStatePath = path.join(EliaAIRoot, '.proxy_enabled');

// Load proxy state from file
function loadProxyState() {
  try {
    trayMenuState.proxyEnabled = fs.existsSync(proxyStatePath);
  } catch (e) {
    trayMenuState.proxyEnabled = false;
  }
  return trayMenuState.proxyEnabled;
}

// Toggle proxy state
function toggleProxy(enabled) {
  try {
    if (enabled) {
      fs.writeFileSync(proxyStatePath, 'enabled\n', 'utf8');
    } else {
      if (fs.existsSync(proxyStatePath)) fs.unlinkSync(proxyStatePath);
    }
    trayMenuState.proxyEnabled = enabled;
    console.log('Proxy:', enabled ? 'ENABLED' : 'DISABLED');
  } catch (e) {
    console.error('toggleProxy:', e.message);
  }
}

// Load current model from file or default
function loadCurrentModel() {
  try {
    if (fs.existsSync(modelSelectionsPath)) {
      const sel = JSON.parse(fs.readFileSync(modelSelectionsPath, 'utf8'));
      if (sel && typeof sel === 'object' && typeof sel['elia'] === 'string' && sel['elia']) {
        const short = sel['elia'].replace(/^opencode\//, '');
        if (SAFE_MODELS.includes(short)) {
          trayMenuState.selectedModel = short;
          return short;
        }
      }
    }
  } catch (e) {
    console.error('Load model selections:', e.message);
  }
  try {
    const modelPath = path.join(EliaAIRoot, '.opencode_model');
    if (fs.existsSync(modelPath)) {
      const model = fs.readFileSync(modelPath, 'utf8').trim();
      if (SAFE_MODELS.includes(model)) {
        trayMenuState.selectedModel = model;
        return model;
      }
    }
  } catch (e) {
    console.error('Load current model:', e.message);
  }
  return 'big-pickle';
}

// Get current scheduler settings — standardEnabled follows the real LaunchAgent plist (not stale .scheduler_state).
function getCurrentCronSettings() {
  const stateFile = path.join(EliaAIRoot, '.scheduler_state');
  const home = process.env.HOME || '/Users/vakandi';
  const launchdPlist = path.join(home, 'Library/LaunchAgents/com.elia.elia-agent.plist');
  const morningPlist = path.join(home, 'Library/LaunchAgents/com.elia.elia-agent-morning.plist');
  const plistInstalled = fs.existsSync(launchdPlist);

  const parseIntervalFromPlist = (content) => {
    const intervalMatch = content.match(/<key>StartInterval<\/key>\s*<integer>(\d+)<\/integer>/);
    const calendarMatch = content.match(/<key>StartCalendarInterval<\/key>/);
    const minuteMatches = content.match(/<key>Minute<\/key><integer>(\d+)<\/integer>/g);
    let detectedInterval = '1h';
    if (intervalMatch) {
      const sec = parseInt(intervalMatch[1], 10);
      detectedInterval =
        sec === 1800 ? '30min' :
        sec === 1200 ? '20min' :
        sec === 7200 ? '2h' :
        sec === 10800 ? '3h' :
        sec === 14400 ? '4h' : '1h';
    } else if (calendarMatch && minuteMatches) {
      const minutes = minuteMatches.map((m) => parseInt(m.match(/\d+/)[0], 10));
      if (minutes.includes(0) && minutes.includes(30)) detectedInterval = '30min';
      else if (minutes.includes(0) && minutes.includes(20) && minutes.includes(40)) detectedInterval = '20min';
      else if (minutes.length === 1 && minutes[0] === 0) detectedInterval = '1h';
    }
    return detectedInterval;
  };

  try {
    let fromFile = null;
    if (fs.existsSync(stateFile)) {
      const state = fs.readFileSync(stateFile, 'utf8');
      const lines = state.trim().split('\n');
      const settings = {};
      lines.forEach((line) => {
        const [key, value] = line.split('=');
        if (key && value !== undefined) {
          settings[key] = value;
        }
      });
      fromFile = {
        morningEnabled: settings.morningEnabled === 'true',
        morningHour: parseInt(settings.morningHour, 10) || 10,
        interval: settings.interval || '1h',
        startHour: parseInt(settings.startHour, 10) || 11,
        endHour: parseInt(settings.endHour, 10) || 21,
      };
    }

    if (plistInstalled) {
      const content = fs.readFileSync(launchdPlist, 'utf8');
      const detectedInterval = parseIntervalFromPlist(content);
      return {
        morningEnabled: fs.existsSync(morningPlist),
        morningHour: fromFile?.morningHour ?? 10,
        standardEnabled: true,
        interval: fromFile?.interval || detectedInterval,
        startHour: fromFile?.startHour ?? 9,
        endHour: fromFile?.endHour ?? 23,
      };
    }

    if (fromFile) {
      return {
        morningEnabled: fs.existsSync(morningPlist),
        morningHour: fromFile.morningHour,
        standardEnabled: false,
        interval: fromFile.interval,
        startHour: fromFile.startHour,
        endHour: fromFile.endHour,
      };
    }
  } catch (e) {
    console.error('Error reading scheduler state:', e.message);
  }

  return {
    morningEnabled: fs.existsSync(morningPlist),
    morningHour: 10,
    standardEnabled: false,
    interval: '1h',
    startHour: 11,
    endHour: 21,
  };
}


function updateTrayMenu() {
  // Load current model
  const currentModel = loadCurrentModel();
  trayMenuState.selectedModel = currentModel;
  
  // Load proxy state
  loadProxyState();
  
  // Get current cron settings
  const cronSettings = getCurrentCronSettings();
  
  const models = [
    { id: 'x-preview-f-free', label: 'Ox Alpha Free (Unlimited)' },
    { id: 'big-pickle', label: 'Big Pickle' },
    { id: 'deepseek-v4-flash-free', label: 'DeepSeek V4 Flash' },
    { id: 'mimo-v2.5-free', label: 'MIMO V2.5' },
    { id: 'nemotron-3-ultra-free', label: 'Nemotron 3 Ultra' }
  ];

  const modelSubmenu = models.map(m => ({
    label: m.label,
    type: 'checkbox',
    checked: trayMenuState.selectedModel === m.id,
    click: () => {
      trayMenuState.selectedModel = m.id;
      writeModelForCron(m.id);
      mainWindow?.webContents.executeJavaScript(`
        localStorage.setItem('selectedModel', '${m.id}');
        if (typeof selectModel === 'function') selectModel('${m.id}');
      `);
      updateTrayMenu();
    }
  }));
  
  // Morning cron submenu
  const morningHours = [];
  for (let h = 6; h <= 12; h++) {
    morningHours.push({
      label: `${h}:00`,
      type: 'radio',
      checked: cronSettings.morningHour === h,
      click: () => {
        try {
          execSync(`/bin/zsh ${EliaAIRoot}/scripts/manage_cron.sh install-morning --morning-hour ${h} >/dev/null 2>&1`, { encoding: 'utf8' });
        } catch (e) {}
        updateTrayMenu();
      }
    });
  }
  
  const morningSubmenu = [
    {
      label: isProcessingMorning ? '⏳ Morning Cron (Processing...)' : 'Morning Cron',
      type: 'checkbox',
      checked: cronSettings.morningEnabled,
      click: () => {
        if (isProcessingMorning) return;
        isProcessingMorning = true;
        updateTrayMenu();
        setTimeout(() => {
          try {
            if (cronSettings.morningEnabled) {
              execSync(`/bin/zsh ${EliaAIRoot}/scripts/manage_cron.sh uninstall-morning >/dev/null 2>&1`, { encoding: 'utf8' });
            } else {
              execSync(`/bin/zsh ${EliaAIRoot}/scripts/manage_cron.sh install-morning --morning-hour ${cronSettings.morningHour} >/dev/null 2>&1`, { encoding: 'utf8' });
            }
          } catch (e) {}
          isProcessingMorning = false;
          updateTrayMenu();
        }, 100);
      }
    },
    { type: 'separator' },
    { label: 'Hour:', enabled: false },
    ...morningHours
  ];
  
  // Standard cron submenu - intervals
  const intervals = ['20min', '30min', '1h', '2h', '3h', '4h'];
  const intervalSubmenu = intervals.map(i => ({
    label: i,
    type: 'radio',
    checked: cronSettings.interval === i,
    click: () => {
      try {
        execSync(`/bin/zsh ${EliaAIRoot}/scripts/manage_cron.sh install --interval ${i} --start ${cronSettings.startHour} --end ${cronSettings.endHour} >/dev/null 2>&1`, { encoding: 'utf8' });
      } catch (e) {}
      updateTrayMenu();
    }
  }));
  
  // Standard cron submenu - hours
  const hoursSubmenu = [];
  for (let start = 8; start <= 18; start++) {
    for (let end = start + 2; end <= 22; end += 2) {
      hoursSubmenu.push({
        label: `${start}:00 - ${end}:00`,
        type: 'radio',
        checked: cronSettings.startHour === start && cronSettings.endHour === end,
        click: () => {
          try {
            execSync(`/bin/zsh ${EliaAIRoot}/scripts/manage_cron.sh install --interval ${cronSettings.interval} --start ${start} --end ${end} >/dev/null 2>&1`, { encoding: 'utf8' });
          } catch (e) {}
          updateTrayMenu();
        }
      });
    }
  }
  
  const cronSubmenu = [
    {
      label: 'Cron Job',
      type: 'checkbox',
      checked: cronSettings.standardEnabled,
      click: () => {
        isProcessingCron = true;
        updateTrayMenu();
        setTimeout(() => {
          try {
            if (cronSettings.standardEnabled) {
              execSync(`/bin/zsh ${EliaAIRoot}/scripts/manage_cron.sh uninstall >/dev/null 2>&1`, { encoding: 'utf8' });
            } else {
              execSync(`/bin/zsh ${EliaAIRoot}/scripts/manage_cron.sh install --interval ${cronSettings.interval} --start ${cronSettings.startHour} --end ${cronSettings.endHour} >/dev/null 2>&1`, { encoding: 'utf8' });
            }
          } catch (e) {}
          isProcessingCron = false;
          updateTrayMenu();
        }, 100);
      }
    },
    { type: 'separator' },
    { label: 'Interval:', enabled: false },
    ...intervalSubmenu,
    { type: 'separator' },
    { label: 'Hours:', enabled: false },
    ...hoursSubmenu
  ];
  
  const menu = Menu.buildFromTemplate([
    { label: 'Afficher', click: () => mainWindow?.show() },
    { label: 'Masquer', click: () => mainWindow?.hide() },
    { type: 'separator' },
    {
      label: 'Morning Routine',
      submenu: [
        {
          label: 'Run Morning Routine',
          click: () => showMorningRoutineConfirmation()
        },
        { type: 'separator' },
        {
          label: 'Morning Speak',
          click: () => runMorningSpeak()
        },
        { type: 'separator' },
        ...morningSubmenu
      ]
    },
    {
      label: isProcessingCron ? '⏳ Cron Job (Processing...)' : 'Cron Job',
      submenu: cronSubmenu
    },
    { type: 'separator' },
    {
      label: 'Modèle',
      submenu: modelSubmenu
    },
    { type: 'separator' },
    {
      label: 'Proxy',
      submenu: [
        {
          label: 'Enable Proxy',
          type: 'checkbox',
          checked: trayMenuState.proxyEnabled,
          click: () => {
            const newState = !trayMenuState.proxyEnabled;
            if (newState && !isProxychainsInstalled()) {
              showProxyErrorPopup();
              return;
            }
            toggleProxy(newState);
            updateTrayMenu();
          }
        },
        {
          label: trayMenuState.proxyEnabled ? 'Status: ENABLED' : 'Status: DISABLED',
          enabled: false
        }
      ]
    },
    { type: 'separator' },
    { label: 'Quitter', click: () => app.quit() }
  ]);
  tray.setContextMenu(menu);
}

function saveTraySettings() {
  try {
    const currentConfig = JSON.parse(fs.readFileSync(configPath, 'utf8'));
    currentConfig.display = currentConfig.display || {};
    currentConfig.display.statusLive = trayMenuState.statusLive;
    currentConfig.display.voiceTrigger = trayMenuState.voiceTrigger;
    fs.writeFileSync(configPath, JSON.stringify(currentConfig, null, 2) + '\n', 'utf8');
  } catch (e) {
    console.error('Save tray settings:', e.message);
  }
}

function createTray() {
  const iconPath = '/Users/vakandi/EliaAI/ui_electron/imgs/electronui.png';
  let trayIcon = nativeImage.createFromPath(iconPath);
  if (trayIcon.isEmpty()) {
    trayIcon = nativeImage.createEmpty();
  }
  tray = new Tray(trayIcon, 'EliaUI-Tray');
  tray.setToolTip('EliaUI');
  console.log('Tray created with title:', tray.getTitle ? tray.getTitle() : 'N/A');
  tray.setToolTip('EliaUI');
  tray.setTitle('Elia');
  console.log('Tray title set to: Elia');
  updateTrayMenu();
  tray.on('click', () => {
    mainWindow?.isVisible() ? mainWindow.hide() : mainWindow?.show();
  });
}

// Morning Routine Confirmation Popup
function showMorningRoutineConfirmation() {
  if (morningPopup && !morningPopup.isDestroyed()) {
    morningPopup.focus();
    return;
  }
  
  const display = screen.getPrimaryDisplay();
  const { width: screenWidth, height: screenHeight } = display.workAreaSize;
  
  const popupW = 750;
  const popupH = 525;
  const x = Math.round((screenWidth - popupW) / 2);
  const y = Math.round((screenHeight - popupH) / 2);
  
  morningPopup = new BrowserWindow({
    width: popupW,
    height: popupH,
    x,
    y,
    frame: false,
    transparent: true,
    resizable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    backgroundColor: '#00000000',
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    }
  });
  
  morningPopup.loadFile(path.join(__dirname, '..', 'morning-popup.html'));
  
  morningPopup.on('closed', () => {
    morningPopup = null;
  });
}

// Morning Speak - trigger cron job with morning briefing prompt
function runMorningSpeak() {
  const { exec } = require('child_process');
  const fs = require('fs');
  const path = require('path');
  
  const eliaAI = '/Users/vakandi/EliaAI';
  const promptFile = path.join(eliaAI, '.morning_briefing_prompt.txt');
  
  const morningPrompt = `MORNING BRIEFING - COMPREHENSIVE DAILY UPDATE:

You are Elia's morning briefing assistant. Your task is to gather ALL relevant information and provide a complete spoken briefing to Wael.

CRITICAL: You must SPEAK to Wael during the ENTIRE process, not just at the end. Use elia-voxtral-speak throughout.

SPEAK AT THESE MOMENTS:
1. AT THE START: "Salut Wael, je démarre le briefing matinal. Je check tout ça."
2. AFTER EACH CHECK: "Je finishes de checker [Google Calendar / WhatsApp / Telegram / Jira], je te donne le point."
3. BEFORE SENDING TASKS: "Je t'ajoute [X] tâches sur ton téléphone."
4. AT THE END: "C'est bon Wael, voici le résumé complet de la matinée."

MUST DO:
1. CHECK GOOGLE CALENDAR: Use gws-workspace list-events to get today's meetings and events
2. CHECK TELEGRAM: Read recent messages from Watson IA group (chat ID: -5148361692)
3. CHECK WHATSAPP: Read B2LUXE BUSINESS (120363408208578679@g.us) and COBOU PowerRangers (120363420711538035@g.us)
4. CHECK JIRA: Get pending tickets for BEN, COBOUAGENC, ZOVAPANEL, TIKYT
5. CHECK MEMORY FILES: Read /Users/vakandi/EliaAI/memory/*.md for important context
6. GATHER BUSINESS UPDATES: Status of all 8 businesses
7. IDENTIFY ACTION ITEMS: What needs Wael's attention today?
8. IDENTIFY WAITING ON: What are team members waiting for?

AFTER GATHERING ALL INFO:
- Use gws-workspace create-task to add any important tasks to Wael's phone
- Use gws-workspace create-event to add any meetings to calendar if missing

IMPORTANT: Speak at EACH step using elia-voxtral-speak (fast, French) → fallback: elia-speak`;
  
  fs.writeFileSync(promptFile, morningPrompt, 'utf8');
  
  const cmd = `cd ${eliaAI} && EXTRA_PROMPT_FILE=${promptFile} /bin/zsh ${eliaAI}/scripts/start_agents.sh`;
  const script = [
    'tell application "Terminal" to activate',
    'tell application "Terminal" to do script ' + JSON.stringify(cmd),
    'delay 0.3',
    'tell application "Terminal" to set bounds of front window to {100, 50, 600, 900}'
  ].map(s => `-e ${JSON.stringify(s)}`).join(' ');
  
  require('child_process').exec(`osascript ${script}`, (error, stdout, stderr) => {
    if (error) {
      console.error('Morning Speak error:', error.message);
      try { fs.unlinkSync(promptFile); } catch(e) {}
    }
  });
}

// Manual Cron Confirmation Popup
function showCronConfirmation() {
  if (cronPopup && !cronPopup.isDestroyed()) {
    cronPopup.focus();
    return;
  }
  
  const display = screen.getPrimaryDisplay();
  const { width: screenWidth, height: screenHeight } = display.workAreaSize;
  
  const popupW = 750;
  const popupH = 525;
  const x = Math.round((screenWidth - popupW) / 2);
  const y = Math.round((screenHeight - popupH) / 2);
  
  cronPopup = new BrowserWindow({
    width: popupW,
    height: popupH,
    x,
    y,
    frame: false,
    transparent: true,
    resizable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    backgroundColor: '#00000000',
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    }
  });
  
  cronPopup.loadFile(path.join(__dirname, '..', 'cron-popup.html'));
  
  cronPopup.on('closed', () => {
    cronPopup = null;
  });
}

// Subworker Manager Popup
function showSubworkerPopupWindow() {
  if (subworkerPopup && !subworkerPopup.isDestroyed()) {
    subworkerPopup.focus();
    return;
  }

  const display = screen.getPrimaryDisplay();
  const { width: screenWidth, height: screenHeight } = display.workAreaSize;

  const popupW = 950;
  const popupH = 680;
  const x = Math.round((screenWidth - popupW) / 2);
  const y = Math.round((screenHeight - popupH) / 2);

  subworkerPopup = new BrowserWindow({
    width: popupW,
    height: popupH,
    x,
    y,
    frame: false,
    transparent: true,
    resizable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    backgroundColor: '#00000000',
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    }
  });

  subworkerPopup.loadFile(path.join(__dirname, '..', 'subworker-popup.html'));

  subworkerPopup.on('closed', () => {
    subworkerPopup = null;
  });
}

function showSchedulePickerWindow(agentName, currentSchedule) {
  if (schedulePickerPopup && !schedulePickerPopup.isDestroyed()) {
    schedulePickerPopup.focus();
    return;
  }

  const display = screen.getPrimaryDisplay();
  const { width: screenWidth, height: screenHeight } = display.workAreaSize;

  const popupW = 420;
  const popupH = 520;
  const x = Math.round((screenWidth - popupW) / 2);
  const y = Math.round((screenHeight - popupH) / 2);

  schedulePickerPopup = new BrowserWindow({
    width: popupW,
    height: popupH,
    x,
    y,
    frame: false,
    transparent: false,
    resizable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    backgroundColor: '#0a0f1a',
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    }
  });

  schedulePickerPopup.loadFile(path.join(__dirname, '..', 'schedule-picker.html'));

  schedulePickerPopup.webContents.on('did-finish-load', () => {
    schedulePickerPopup.webContents.send('schedule-picker-init', {
      agentName,
      currentSchedule
    });
  });

  schedulePickerPopup.on('closed', () => {
    schedulePickerPopup = null;
  });
}

function showWelcomePopup() {
  if (welcomeShown) return;
  welcomeShown = true;
  
  const display = screen.getPrimaryDisplay();
  const { width: screenWidth, height: screenHeight } = display.workAreaSize;
  
  const popupW = 750;
  const popupH = 525;
  const x = Math.round((screenWidth - popupW) / 2);
  const y = Math.round((screenHeight - popupH) / 2);
  
  welcomePopup = new BrowserWindow({
    width: popupW,
    height: popupH,
    x,
    y,
    frame: false,
    transparent: true,
    resizable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    backgroundColor: '#00000000',
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    }
  });
  
  welcomePopup.loadFile(path.join(__dirname, '..', 'welcome-popup.html'));
  
  setTimeout(() => {
    if (welcomePopup && !welcomePopup.isDestroyed()) {
      welcomePopup.close();
    }
  }, 10000);
  
  welcomePopup.on('closed', () => {
    welcomePopup = null;
  });
}

// Proxy Error Popup
function showProxyErrorPopup() {
  if (proxyPopup && !proxyPopup.isDestroyed()) {
    proxyPopup.focus();
    return;
  }
  
  const display = screen.getPrimaryDisplay();
  const { width: screenWidth, height: screenHeight } = display.workAreaSize;
  
  const popupW = 600;
  const popupH = 300;
  const x = Math.round((screenWidth - popupW) / 2);
  const y = Math.round((screenHeight - popupH) / 2);
  
  proxyPopup = new BrowserWindow({
    width: popupW,
    height: popupH,
    x,
    y,
    frame: false,
    transparent: true,
    resizable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    backgroundColor: '#00000000',
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    }
  });
  
  proxyPopup.loadFile(path.join(__dirname, '..', 'proxy-error-popup.html'));
  
  proxyPopup.on('closed', () => {
    proxyPopup = null;
  });
}

// Check if proxychains4 is installed
function isProxychainsInstalled() {
  try {
    execSync('which proxychains4 2>/dev/null', { encoding: 'utf8' });
    return true;
  } catch (e) {
    return false;
  }
}

function startTelegramBot() {
  const botDir = path.join(EliaAIRoot, 'integrations', 'telegram-opencode-bot');
  const botScript = path.join(botDir, 'dist', 'cli.js');
  if (!fs.existsSync(botScript)) {
    console.log('Telegram bot: dist missing, run build in integrations/telegram-opencode-bot');
    return;
  }
  const envPath = path.join(botDir, '.env');
  if (fs.existsSync(envPath)) {
    const envContent = fs.readFileSync(envPath, 'utf8');
    if (!/TELEGRAM_BOT_TOKENS\s*=\s*.+/.test(envContent)) {
      return;
    }
  } else {
    return;
  }
  try {
    telegramBotProcess = spawn('node', [botScript], {
      cwd: botDir,
      env: { ...process.env, ELIA_HELPER_DIR: EliaAIRoot },
      stdio: ['ignore', 'pipe', 'pipe']
    });
    telegramBotProcess.on('error', (err) => {
      console.error('Telegram bot error:', err.message);
      telegramBotProcess = null;
    });
    telegramBotProcess.on('exit', (code, signal) => {
      telegramBotProcess = null;
    });
  } catch (e) {
    console.error('Telegram bot spawn failed:', e.message);
  }
}

function startDiscordBot() {
  const botDir = path.join(EliaAIRoot, 'integrations', 'elia-discord-bot');
  const botScript = path.join(botDir, 'bot.py');
  if (!fs.existsSync(botScript)) {
    console.log('Discord bot: bot.py not found');
    return;
  }
  const envPath = path.join(botDir, '.env');
  if (fs.existsSync(envPath)) {
    const envContent = fs.readFileSync(envPath, 'utf8');
    if (!/DISCORD_BOT_TOKEN\s*=\s*.+/.test(envContent)) {
      console.log('Discord bot: DISCORD_BOT_TOKEN not set in .env');
      return;
    }
  } else {
    console.log('Discord bot: .env not found');
    return;
  }
  try {
    const venvPython = path.join(botDir, 'venv', 'bin', 'python3');
    const pythonBin = fs.existsSync(venvPython) ? venvPython : 'python3';
    discordBotProcess = spawn(pythonBin, [botScript], {
      cwd: botDir,
      env: { ...process.env },
      stdio: ['ignore', 'pipe', 'pipe']
    });
    discordBotProcess.stdout.on('data', (data) => {
      console.log('[Discord]', data.toString().trim());
    });
    discordBotProcess.stderr.on('data', (data) => {
      console.error('[Discord Error]', data.toString().trim());
    });
    discordBotProcess.on('error', (err) => {
      console.error('Discord bot error:', err.message);
      discordBotProcess = null;
    });
    discordBotProcess.on('exit', (code, signal) => {
      discordBotProcess = null;
    });
  } catch (e) {
    console.error('Discord bot spawn failed:', e.message);
  }
}

app.whenReady().then(() => {
  console.log('EliaUI starting...');
  app.setName('EliaUI');
  
  const template = [
    {
      label: 'EliaUI',
      submenu: [
        { label: 'About EliaUI', role: 'about' },
        { type: 'separator' },
        { label: 'Services', role: 'services' },
        { type: 'separator' },
        { label: 'Hide EliaUI', role: 'hide' },
        { label: 'Hide Others', role: 'hideOthers' },
        { label: 'Show All', role: 'unhide' },
        { type: 'separator' },
        { label: 'Quit EliaUI', role: 'quit' }
      ]
    },
    {
      label: 'Edit',
      submenu: [
        { label: 'Undo', role: 'undo' },
        { label: 'Redo', role: 'redo' },
        { type: 'separator' },
        { label: 'Cut', role: 'cut' },
        { label: 'Copy', role: 'copy' },
        { label: 'Paste', role: 'paste' },
        { label: 'Select All', role: 'selectAll' }
      ]
    },
    {
      label: 'View',
      submenu: [
        { label: 'Reload', role: 'reload' },
        { label: 'Force Reload', role: 'forceReload' },
        { label: 'Toggle DevTools', role: 'toggleDevTools' },
        { type: 'separator' },
        { label: 'Actual Size', role: 'resetZoom' },
        { label: 'Zoom In', role: 'zoomIn' },
        { label: 'Zoom Out', role: 'zoomOut' },
        { type: 'separator' },
        { label: 'Toggle Fullscreen', role: 'togglefullscreen' }
      ]
    },
    {
      label: 'Window',
      submenu: [
        { label: 'Minimize', role: 'minimize' },
        { label: 'Zoom', role: 'zoom' },
        { type: 'separator' },
        { label: 'Bring All to Front', role: 'front' }
      ]
    },
    {
      label: 'Help',
      submenu: [
        { label: 'EliaUI Help', role: 'help' }
      ]
    }
  ];
  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
  console.log('Application menu set');
  
  const dockIconPath = path.join(__dirname, '..', 'imgs', 'dock_icon.png');
  if (fs.existsSync(dockIconPath)) {
    app.dock?.setIcon(dockIconPath);
  }
  
  createTray();
  createWindow();
  showWelcomePopup();
  startTelegramBot();
  if (SET_POSITION_FLAG) {
    const display = screen.getPrimaryDisplay();
    const { x: ox, y: oy, width: dw, height: dh } = display.bounds;
    setPositionOverlay = new BrowserWindow({
      x: ox,
      y: oy,
      width: dw,
      height: dh,
      frame: false,
      transparent: true,
      alwaysOnTop: true,
      skipTaskbar: true,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true,
        preload: path.join(__dirname, 'set-position-preload.js')
      }
    });
    setPositionOverlay.setIgnoreMouseEvents(false);
    setPositionOverlay.loadFile(path.join(__dirname, 'set-position-overlay.html'));
    setPositionOverlay.on('closed', () => { setPositionOverlay = null; });
  }
});

app.on('window-all-closed', (e) => e.preventDefault());
app.on('before-quit', () => {
  if (ntfyStream) try { ntfyStream.destroy(); } catch (e) {}
  if (agentStatusInterval) clearInterval(agentStatusInterval);
  if (telegramBotProcess) {
    telegramBotProcess.kill('SIGTERM');
    telegramBotProcess = null;
  }
  if (discordBotProcess) {
    discordBotProcess.kill('SIGTERM');
    discordBotProcess = null;
  }
});
