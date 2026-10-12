const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('elia', {
  onConfig:      (cb) => ipcRenderer.on('config-updated', (_e, data) => cb(data)),
  onMessage:     (cb) => ipcRenderer.on('ntfy-message',   (_e, data) => cb(data)),
  onAgentStatus:     (cb) => ipcRenderer.on('agent-status',     (_e, data) => cb(data)),
  onRecordingStarted: (cb) => ipcRenderer.on('recording-started', (_e) => cb && cb()),
  onRecordingStopped: (cb) => ipcRenderer.on('recording-stopped', (_e) => cb && cb()),
  onStatusLiveToggle: (cb) => ipcRenderer.on('status-live-toggle', (_e, enabled) => cb(enabled)),
  onVoiceTriggerToggle: (cb) => ipcRenderer.on('voice-trigger-toggle', (_e, enabled) => cb(enabled)),
  onOpencodeContext: (cb) => ipcRenderer.on('opencode-context', (_e, data) => cb(data)),
  getConfig:       ()   => ipcRenderer.send('get-config'),
  hideWindow:      ()   => ipcRenderer.send('hide-window'),
  quitApp:         ()   => ipcRenderer.send('quit-app'),
  executeCommand:  ()   => ipcRenderer.send('execute-elia-command'),
  executeMiniCommand: () => ipcRenderer.send('execute-mini-orb'),
  stopRecording:   ()   => ipcRenderer.send('stop-recording'),
  setIgnoreMouseEvents: (ignore, options) => ipcRenderer.send('set-ignore-mouse-events', ignore, options),
  saveModelForCron: (model) => ipcRenderer.send('save-model-for-cron', model),
});

contextBridge.exposeInMainWorld('electronAPI', {
  send: (channel, data) => ipcRenderer.send(channel, data),
  on: (channel, callback) => {
    ipcRenderer.on(channel, (_event, ...args) => callback(...args));
  },
  /** Main replies to get-toggle-states with channel toggle-states */
  onToggleStates: (callback) => {
    ipcRenderer.on('toggle-states', (_event, states) => callback(states));
  },
  onSubworkers: (callback) => {
    ipcRenderer.on('subworkers-list', (_event, data) => callback(data));
  },
  onSubworkerToggled: (callback) => {
    ipcRenderer.on('subworker-toggled', (_event, result) => callback(result));
  },
  removeAllListeners: (channel) => ipcRenderer.removeAllListeners(channel),
});

contextBridge.exposeInMainWorld('popupAPI', {
  controlUrl: process.env.ELIA_CONTROL_URL || 'http://127.0.0.1:5680',
  runMorning: () => ipcRenderer.send('run-morning-routine'),
  runCron: () => ipcRenderer.send('run-cron-manual'),
  close: () => ipcRenderer.send('close-popup'),
  openUrl: (url) => ipcRenderer.send('open-url', url),
  openLogsTerminal: () => ipcRenderer.send('open-logs-terminal'),
  openSubworkerLogs: (name) => ipcRenderer.send('open-subworker-logs', name),
  openSubworkerLogFile: (path) => ipcRenderer.send('open-subworker-log-file', path),
  getSubworkerRuns: (name) => ipcRenderer.send('get-subworker-runs', name),
  openSchedulePicker: (name, currentSchedule) => ipcRenderer.send('open-schedule-picker', name, currentSchedule),
  runSubworkerNow: (name, model) => ipcRenderer.send('run-subworker-now', name, model),
  getModelSelections: () => ipcRenderer.invoke('get-model-selections'),
  saveModelSelection: (name, model) => ipcRenderer.send('save-model-selection', name, model),
  setMainAgent: (name) => ipcRenderer.send('set-main-agent', name),
});

// Extend electronAPI with subworker runs reply
contextBridge.exposeInMainWorld('subworkerRunsAPI', {
  onSubworkerRuns: (callback) => {
    ipcRenderer.on('subworker-runs', (_event, data) => callback(data));
  },
});

contextBridge.exposeInMainWorld('schedulePickerAPI', {
  onInit: (callback) => {
    ipcRenderer.on('schedule-picker-init', (_event, data) => callback(data));
  },
  saveSchedule: (agentName, scheduleConfig) => {
    ipcRenderer.send('save-subworker-schedule', { agentName, scheduleConfig });
  },
  onSaved: (callback) => {
    ipcRenderer.on('schedule-saved', (_event, result) => callback(result));
  },
  close: () => ipcRenderer.send('close-popup'),
});
