#!/bin/bash
# ============================================================
#  ntfy-watch.sh — Affiche les notifications ntfy.sh en temps réel
#  Usage : ./ntfy-watch.sh [topic] [server]
# ============================================================

# ── Configuration ────────────────────────────────────────────
TOPIC="${1:-mon-topic}"                      # ton topic ntfy
SERVER="${2:-https://ntfy.sh}"              # ou ton serveur self-hosted
# Si ton serveur nécessite une auth, décommente et remplis :
# AUTH="Bearer ton-token-ici"
# AUTH="Basic $(echo -n 'user:pass' | base64)"
# ─────────────────────────────────────────────────────────────

ENDPOINT="${SERVER}/${TOPIC}/json"

# Couleurs
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

# Priorités → labels colorés
priority_label() {
  case "$1" in
    1|min)      echo -e "${CYAN}[MIN]${RESET}" ;;
    2|low)      echo -e "${GREEN}[LOW]${RESET}" ;;
    3|default)  echo -e "[NRM]" ;;
    4|high)     echo -e "${YELLOW}[HIGH]${RESET}" ;;
    5|urgent|max) echo -e "${RED}[URG]${RESET}" ;;
    *)          echo -e "[---]" ;;
  esac
}

# Vérifie les dépendances
for cmd in curl jq; do
  if ! command -v "$cmd" &>/dev/null; then
    echo -e "${RED}Erreur : '$cmd' est requis. Installe-le avec : brew install $cmd${RESET}"
    exit 1
  fi
done

echo -e "${BOLD}🔔 ntfy-watch — Écoute du topic :${RESET} ${CYAN}${TOPIC}${RESET} sur ${SERVER}"
echo -e "${BOLD}   Appuie sur Ctrl+C pour quitter.${RESET}"
echo "────────────────────────────────────────────────────────"

# Boucle principale : reconnexion automatique si la connexion tombe
while true; do
  curl -sS --no-buffer \
    ${AUTH:+-H "Authorization: $AUTH"} \
    "${ENDPOINT}" | \
  while IFS= read -r line; do
    # Ignore les lignes vides
    [[ -z "$line" ]] && continue

    # Décode le JSON
    EVENT=$(echo "$line" | jq -r '.event // "message"' 2>/dev/null)
    [[ "$EVENT" != "message" ]] && continue   # ignore keepalive / open

    TIMESTAMP=$(echo "$line" | jq -r '.time // empty' 2>/dev/null)
    TITLE=$(echo "$line"     | jq -r '.title // ""' 2>/dev/null)
    MESSAGE=$(echo "$line"   | jq -r '.message // ""' 2>/dev/null)
    PRIORITY=$(echo "$line"  | jq -r '.priority // 3' 2>/dev/null)
    TAGS=$(echo "$line"      | jq -r '(.tags // []) | join(", ")' 2>/dev/null)

    # Formate l'heure
    if [[ -n "$TIMESTAMP" ]]; then
      TIME_STR=$(date -r "$TIMESTAMP" "+%H:%M:%S" 2>/dev/null || \
                 date -d "@$TIMESTAMP" "+%H:%M:%S" 2>/dev/null || \
                 echo "??:??:??")
    else
      TIME_STR=$(date "+%H:%M:%S")
    fi

    PRIO_LABEL=$(priority_label "$PRIORITY")

    # Affichage
    echo -e ""
    echo -e "${BOLD}${TIME_STR}${RESET} ${PRIO_LABEL} ${BOLD}${TITLE:-'(sans titre)'}${RESET}"
    echo -e "   ${MESSAGE}"
    [[ -n "$TAGS" ]] && echo -e "   🏷  ${CYAN}${TAGS}${RESET}"
    echo "────────────────────────────────────────────────────────"

    # Notification macOS native (optionnel)
    osascript -e "display notification \"${MESSAGE}\" with title \"ntfy: ${TITLE:-$TOPIC}\"" 2>/dev/null

  done

  echo -e "${YELLOW}⚠ Connexion perdue, reconnexion dans 5s…${RESET}"
  sleep 5
done
