# MirrorPay Telegram — Anchored Memory

> Dernière mise à jour : 22/05/2026

---

## 📡 Discord — Communication BIDIRECTIONNELLE

⚠️ **Wael (master) parle via ces channels — je DOIS les lire à chaque run**

| Channel | ID | J'envoie | Je lis (ordres de Wael) |
|---------|-----|----------|-------------------------|
| 📊-reports | `1506669264138932406` | Rapport de run | Feedbacks, ajustements |
| 👥-leads | `1506680446442930236` | Fiche prospect | Qualification, questions |
| 👤-clients | `1506680489925152920` | Conversion client | Suivi conversions |
| 📈-pipeline | `1506680536255434754` | Pipeline cumulé (23h) | Ajustement stratégie |
| 💻-dev | `1506669339791724624` | Bugs uniquement | Solutions, priorités |

**Règle : Avant chaque action → lire les 5 channels → répondre à Wael en priorité.**

---

## 🎯 Telegram Channels MirrorPay (TROUVÉS)

**Les 3 channels existent bien — NE PLUS DIRE "à créer" :**

| Langue | Username | ID Telegram | Membres |
|--------|----------|-------------|---------|
| 🇫🇷 Français | **@MirrorPaySaaS** | 3873261591 | 2 |
| 🌍 Global/Anglais | **@MirorPayGlobal** | 3775871427 | 1 |
| 🇨🇳 Chinois | **@MirorPayCN** | 3907183472 | 1 |

**Règle d'invitation :**
- Prospect francophone → `@MirrorPaySaaS`
- Prospect anglophone → `@MirorPayGlobal`
- Prospect chinois → `@MirorPayCN`

---

## 🔧 MCP Tools

| MCP | Usage | Notes |
|-----|-------|-------|
| `telegram-scraper` | **SEUL outil Telegram** - discover, scrape, invite | ✅ Toujours |
| `telegram` (normal) | ❌ **NE JAMAIS UTILISER** | Pas configuré |
| `discord-server-mcp` | Reporting Discord | ✅ Obligatoire après chaque run |
| `mcp-atlassian` | Jira MIRORPAY | ✅ Check avant chaque run |

---

## 🔄 Jira Active Issues

- **MIRORPAY-5** — Run de recrutement précédent (commenté)
- **MIRORPAY-7** — Epic Phase 0 Campaign

---

## ⚠️ Problèmes Connus

1. **Groupes FR = 100% spam** — Dernier message réel novembre 2025. Pas de prospects viables dans dropshippingfrance, shopifyfrance, etc.
2. **`join_group` échoue** — stripemorocco, blackhatworld, shopifyfrance, stripealgeria n'existent pas ou sont restreints
3. **`discover_similar_channels` retourne 0** — Fonctionne seulement sur groupes >10k membres
4. **`uv` était manquant** — Installé via `brew install uv` le 22/05
5. **`mcp-cli` path** : `/Users/vakandi/.local/bin/mcp-cli`

---

## 📊 Statistiques Scraper

- 8 comptes configurés, tous avec sessions
- 0 membres scrapés
- 0 invitations envoyées

---

## ✅ Ce qu'on a appris

1. Les channels MirrorPay existent déjà — **@MirrorPaySaaS** (FR), **@MirorPayGlobal** (EN), **@MirorPayCN** (CN)
2. Les groupes français sont morts (spam uniquement) — viser l'international
3. Le scraper fonctionne mais `discover_similar_channels` est limité aux gros groupes
4. Les 8 comptes scraper sont prêts à l'emploi
