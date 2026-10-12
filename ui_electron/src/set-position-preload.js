const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('setPositionClick', (screenX, screenY) => {
  ipcRenderer.send('set-position-click', screenX, screenY);
});
