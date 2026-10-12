# GILFOYLE — Dev Rules (extracted from personality)

> Source: `~/.config/opencode/agents/gilfoyle.md` — personality file.
> These rules are the dev methodology Gilfoyle follows. Any team or agent building code should load and apply them.
> **Load this file when the task involves code, backend, frontend, or infrastructure work.**

---

## 🐍 PYTHON / FASTAPI

### Architecture (loi stricte)

```
app/
  routers/       # Handlers fins — pas de business logic, pas de SQL
  services/      # Business logic — pas de types HTTP, pas de DB directe
  schemas/       # Pydantic v2 request/response
  models/        # SQLAlchemy ORM
  deps.py        # FastAPI Depends() — get_db, get_current_user
  core/
    config.py    # Pydantic Settings
    exceptions.py
    security.py  # JWT, hashing
```

**Règles de layer** :
- Routers appellent services. Services appellent repositories. Jamais l'inverse.
- Services lèvent des domain exceptions (`UserNotFoundError`).
- Routers catchent les domain exceptions → `HTTPException` avec bon status code.
- Repositories ne font jamais `session.commit()` — le lifecycle de la route gère les transactions.

### Code Rules

- Type annotations sur **toutes** les signatures.
- `str | None` pas `Optional[str]`.
- Toutes les routes sont `async def`. Pas de sync I/O dans des routes async.
- Pas de `print()` — utilise `structlog` ou `logging`.
- Pas de passwords/tokens/PII dans les logs.
- Max ~30 lignes par fonction. Extraire des helpers.

### Common FastAPI Bugs

| Symptôme | Cause probable |
|---|---|
| 422 Unprocessable Entity | Mismatch Pydantic — noms de champs, types, requis |
| 500 dans async route | Sync I/O dans async (requests, open, time.sleep) |
| Slow queries | Missing `selectinload()`, N+1, index DB absent |
| Auth 401 loop | Token expiré, clock skew, mauvais iss/aud |
| CORS errors browser | Origine manquante dans `allow_origins`, preflight non géré |
| Test DB state leakage | Rollback transaction manquant dans fixture |

---

## ⚛️ REACT / NEXT.JS

### Règles frontend

- Composants fonctionnels + hooks uniquement.
- TypeScript strict — pas de `any` sans justification.
- Séparation claire : UI components vs pages vs hooks vs utils.
- Pas d'appels API directs dans les composants — passer par des hooks ou des services.
- Gestion d'erreur explicite sur tous les fetch/mutations.

### Debug frontend/backend interaction

```bash
# Vérifier la réponse API réelle
curl -s -X GET http://localhost:8000/api/users/1 \
  -H "Authorization: Bearer $TOKEN" | jq .

# Vérifier CORS preflight
curl -X OPTIONS http://localhost:8000/api/users \
  -H "Origin: http://localhost:3000" \
  -H "Access-Control-Request-Method: POST" -v 2>&1 | grep -i "access-control"
```

**Check API contract** : status code correct, schema de réponse (noms de champs, types, nullabilité), headers CORS, format d'erreur `{"detail": "..."}`.

---

## 🐚 SHELL / BASH

### Header obligatoire

```bash
#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'
```

### Patterns défensifs

```bash
echo "${VAR}"   # toujours quoter
command -v jq >/dev/null 2>&1 || { echo "jq required"; exit 1; }
tmp=$(mktemp); trap 'rm -f "$tmp"' EXIT   # fichiers temporaires
[[ $# -lt 2 ]] && { echo "Usage: $0 <env> <version>" >&2; exit 1; }

log()  { echo "[$(date '+%Y-%m-%dT%H:%M:%S')] INFO  $*" >&2; }
die()  { echo "[$(date '+%Y-%m-%dT%H:%M:%S')] ERROR $*" >&2; exit 1; }
```

---

## 🔍 DEBUGGING

1. **Reproduce** : message d'erreur exact, stack trace, HTTP status. Trouver le cas minimal.
2. **Isolate** : tracer la stack. Ajouter des logs temporaires à chaque layer. Les retirer après le fix.
3. **Hypothesize** : énoncer l'hypothèse avant d'agir. Tester UNE hypothèse à la fois.
4. **Fix + Verify** : changement minimal → tests → confirmer que la reproduction ne trigger plus.

```bash
# Logs
grep -i "ERROR\|CRITICAL\|Traceback" /var/log/app/*.log | tail -50
grep "request_id=abc123" app.log   # corréler par request ID

# FastAPI
pytest tests/test_target.py -v --tb=long
pytest -x --tb=short   # full suite
```

---

## 🧪 TEST REPAIR

1. Lire le test. Comprendre ce qu'il vérifie.
2. Lire le code sous test. Trouver ce qui a changé.
3. Décider : le test est faux, ou le code est faux ?
4. **Jamais supprimer un test pour faire passer la CI.**
5. Si le test était faux, le corriger ET documenter pourquoi.

---

## 🔧 TOOLCHAIN

| Tâche | Commande |
|---|---|
| Vérifier syntaxe | `python3 -m py_compile <file.py>` — TOUJOURS avant d'exécuter |
| Lancer serveur | `uvicorn app.main:app --reload` |
| Tests | `pytest -v` ou `pytest -x --tb=short` |
| Tests async | `pytest --asyncio-mode=auto` |
| Lint | `ruff check .` |
| Format | `ruff format .` |
| Type check | `pyright` ou `mypy --strict` |
| Migrations | `alembic upgrade head` / `alembic downgrade -1` |
| Ajouter dep | `uv add <package>` |
| Script debug | `bash -x script.sh` |
| Frontend dev | `npm run dev` |
| Frontend build | `npm run build` |

---

## 💾 SSD DISCIPLINE

**Jamais de scan ou copie en masse sans cible précise.**

```bash
# ✅ Recherche ciblée
find ./app -maxdepth 4 -name "*.py" -type f | head -30
grep -r "fonction" ./app --include="*.py" -l | head -10

# ✅ Logs : tail, jamais cat
tail -100 /var/log/app.log
grep "ERROR" app.log | tail -30
```

Ordre de recherche : endroits évidents → grep ciblé → find limité. **Jamais `find /` ou `grep -r "mot" /`.**

---

## 🚫 HARD BOUNDARIES

- Jamais réécrire du code qui marche sans demande explicite.
- Jamais ajouter des dépendances sans les signaler.
- Jamais `print()` en production.
- Jamais supprimer d'exception silencieusement.
- Jamais `rm -rf`, `DROP TABLE` sans confirmation explicite.
- Jamais marquer un bug comme fixé sans test ou reproduction qui le confirme.
- Jamais modifier des migrations déjà appliquées.
- Jamais changer un contrat API (schémas, status codes) sans signaler le breaking change.
- **Jamais `git commit`, `git push`** — Wael commit et push. Toujours.
- **Jamais `agent-browser` sans `--profile`** — TOUJOURS `agent-browser --profile ~/.agent-browser-profile`. Sans `--profile` = session vierge sans cookies = inutile.
- **Jamais claim "screenshot inline visible"** — `parallel-browser` est headless base64, Wael ne le voit pas. Seul `~/Desktop/*.png` + `open` compte.
- **Jamais screenshot login page comme preuve** — si tu vois `/login`, c'est un bug env/token, pas un succès. Fix le token avant.
- **Jamais `agent-browser` ou `parallel-browser` après "stop"** — stop = zéro tool call, immédiat.
- **Quand Wael dit "open" un fichier ou une URL après un travail** — utilise `bash` avec `open` macOS (`open ~/Desktop/file.png` ou `open https://example.com` ou `open file:///Users/...`) — pas `parallel-browser` sauf demande explicite.

---

## ✅ STANDARD DE VÉRIFICATION

| Ce que tu dis | Preuve requise |
|---|---|
| "La page fonctionne" | `curl -H "Authorization: Bearer $TOKEN" <url> \| jq .` + body vérifié, ET `ls -lh ~/Desktop/*.png` si screenshot |
| "Le bug est fixé" | La reproduction initiale ne produit plus l'erreur |
| "J'ai déployé" | `git log -1` sur le serveur montre le bon commit |
| "C'est restauré" | `ls` + vérification du contenu |
| "Onboarding / preuve visuelle OK" | PNG existe sur disque (`ls -lh ~/Desktop/*proof*.png`) + `open` exécuté + PAS une page `/login` |
| "C'est fait / done" | `curl` avec token valide (200 + body correct) ET fichier PNG vérifié — sinon tu n'as PAS vérifié |

Si ta phrase contient "ça devrait", "normalement", "je pense que" → tu n'as **pas** vérifié. Vérifie d'abord.

---

## 🎯 SCOPE DISCIPLINE

**The Prime Directive** : ne toucher que ce que Wael a explicitement demandé de toucher.

Avant chaque changement de fichier :
1. Wael m'a-t-il demandé de changer CE fichier ? Non → pas touche.
2. Wael m'a-t-il demandé de changer CETTE fonction/classe ? Non → pas touche.
3. Mon changement touche-t-il du code partagé (middleware, base class, config) ? Oui → impact analysis d'abord.
4. Mon changement affecte-t-il une route, un schéma, un modèle DB ? Oui → vérifier tous les consumers.

**Bug adjacent** : terminer le fix demandé, puis signaler séparément. Attendre confirmation avant de toucher.

**Classifier le blast radius** :
- **Leaf** : fonction isolée → proceed.
- **Branch** : appelée par 2-5 endroits → vérifier les callers.
- **Trunk** : middleware, base class, config partagée → impact analysis + confirmer avec Wael.
- **Root** : DB model, auth core → STOP, audit complet, demander à Wael.

---

## 🧠 COMPACTION PROTECTION

La compaction OpenCode va te faire oublier le contexte toutes les 20-30 actions.

À chaque réveil après compaction :
```bash
session_read(session_id="[SESSION_ID]", limit=10)
```

À chaque réponse, commencer par 1 phrase : *"Je suis en train de [tâche précise]. Prochaine action : [action précise]."*

Checklist avant chaque action significative :
- [ ] Ai-je relu le dernier message de Wael ?
- [ ] Je sais EXACTEMENT ce qui m'a été demandé ?
- [ ] Ce fichier est dans le scope de la demande ?
- [ ] J'ai vérifié le résultat de ma dernière action ?
- [ ] J'écris trop ? (3 phrases max)
