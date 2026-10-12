# 🔵 Elia ntfy — Interface HUD style Iron Man

Interface flottante Elia sur macOS qui affiche les notifications ntfy.sh en temps réel.

## 📦 Installation

### Prérequis
- Node.js (v18+) → https://nodejs.org
- npm (inclus avec Node.js)

### 1. Installer les dépendances

```bash
cd elia-ntfy
npm install
```

### 2. Configurer ton topic ntfy

Édite `config.json` :

```json
{
  "ntfy": {
    "server": "https://ntfy.sh",
    "topic": "MON-TOPIC-ICI",
    "token": ""          ← si serveur privé avec auth
  }
}
```

### 3. Lancer

```bash
npm start
```

---

## ⚙️ Configuration (`config.json`)

| Champ | Description |
|-------|-------------|
| `identity.name` | Nom affiché en haut (ex: Elia) |
| `identity.subtitle` | Sous-titre |
| `stats[]` | Cases de stats (CPU, RAM, etc.) |
| `menu[]` | Items du menu avec sous-éléments |
| `ntfy.server` | URL du serveur ntfy |
| `ntfy.topic` | Ton topic ntfy |
| `ntfy.token` | Token auth (optionnel) |

Le fichier `config.json` est **surveillé en temps réel** — modifie-le sans redémarrer l'app.

---

## 🔔 Envoyer une notification de test

```bash
# Simple
curl -d "Hello Elia" ntfy.sh/MON-TOPIC

# Avec titre et priorité
curl \
  -H "Title: Déploiement" \
  -H "Priority: high" \
  -H "Tags: rocket,server" \
  -d "Build terminé avec succès" \
  ntfy.sh/MON-TOPIC
```

---

## 🎨 Priorités et couleurs

| Valeur | Couleur | Usage |
|--------|---------|-------|
| 1 (min) | Gris | Info silencieuse |
| 2 (low) | Vert | Succès |
| 3 (default) | Cyan | Normal |
| 4 (high) | Jaune | Attention |
| 5 (urgent) | Rouge | Critique |

---

## 🖥 Comportement UI

- L'orbe **pulse** à chaque nouvelle notification
- Les bulles **glissent** depuis la droite
- **Clic** sur une bulle = dismiss
- Auto-dismiss après **8 secondes**
- **Reconnexion automatique** si ntfy se déconnecte
- Le **point vert** en haut à gauche = connexion active
- **Barre de titre drag** pour déplacer la fenêtre
- **Croix rouge** pour masquer (reste dans la barre de menu)

---

## 📍 Démarrage automatique (macOS)

Créer un LaunchAgent :

```bash
mkdir -p ~/Library/LaunchAgents
cat > ~/Library/LaunchAgents/com.elia.ntfy.plist << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.elia.ntfy</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/node</string>
    <string>/chemin/vers/elia-ntfy/node_modules/.bin/electron</string>
    <string>/chemin/vers/elia-ntfy</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
EOF

launchctl load ~/Library/LaunchAgents/com.elia.ntfy.plist
```
