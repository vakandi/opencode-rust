#!/bin/bash

echo "🚀 Démarrage EliaUI en mode DEBUG MCP..."

# Arrêter les processus existants
echo "📋 Arrêt des processus Electron existants..."
pkill -f "electron/cli.js" 2>/dev/null || true
sleep 2

# Démarrer en mode debug avec accès distant
echo "🔧 Démarrage Electron avec remote debugging..."
cd "$(dirname "$0")"
ELECTRON_ENABLE_LOGGING=1 npm run start:debug

echo "✅ EliaUI lancé en mode debug"
echo "📡 Port remote debugging: 9222"
echo "🔍 Port inspect: 9223"
echo ""
echo "🎯 Pour accéder via Playwright MCP:"
echo "   mcp9_browser_navigate --url http://localhost:9222"
echo ""
echo "🌐 Pour accéder via Chrome:"
echo "   chrome://inspect"
echo "   Puis ouvrez: chrome-devtools://devtools/bundled/inspector.html?ws=localhost:9223"
echo ""
echo "⚡ Pour tester les badges dans la console:"
echo "   window.testBadges()"
echo "   selectModel('big-pickle')"
echo "   document.getElementById('badge-bigpickle').click()"
