# État du travail — OpenCode Rust SubWorker

**Date :** 2026-10-12 (mis à jour)
**Objectif :** faire fonctionner l'agent Refund Hunter sur le serveur Rust, en direct.

> Documentation de référence : **SUBWORKERS_SYSTEM.md** (architecture, config, permissions, bugs connus).
> Règles de travail : **RULES.md**. Plan d'implémentation : **PLAN.md**.
> Ce fichier ne garde que l'état du travail.

---

## TL;DR pour Wael

L'agent fonctionne. Il utilise ses outils et répond pour de vrai.
Le livestream est vérifié. Les permissions sont à moitié en place — `bash` passe encore.

---

## Réparé et vérifié (avec un vrai run)

### 1. L'inférence était morte (403)
Le serveur répondait `200` sur `/health` — donc il paraissait sain. Mais chaque appel au modèle échouait.

**Pourquoi :** dès qu'une clé API était présente, le serveur passait par la mauvaise porte (`openai_compatible`) au lieu de la bonne (`zen`). Cette mauvaise porte n'envoie pas l'empreinte que Zen exige. Résultat : `403 FreeTierError` sur tout.

### 2. L'agent ne recevait jamais les résultats de ses outils
C'était le bug le plus grave. L'agent croyait que ses recherches revenaient vides, relançait en boucle, et finissait par dire « I'll search now... » sans jamais rien trouver.

**Pourquoi :** les résultats étaient envoyés dans un format que le serveur OpenAI ignore en silence. Aucune erreur affichée — juste rien.

### 3. La boucle s'arrêtait trop tôt
Le modèle dit « je vais chercher » **et** demande l'outil dans le même message. Le code voyait du texte et arrêtait. L'agent n'avait jamais le temps de lire ce qu'il venait de trouver.

### 4. Bonus : deux bugs trouvés en relecture
- Les résultats des sous-agents étaient jetés completely.
- Les images envoyées étaient supprimées en silence.
- Un message malformé faisait rejeter toute la requête.

---

## Preuve que ça marche

Test réel : `webfetch` bloqué par CloudFront → l'agent bascule sur `curl`, identifie le pare-feu AWS, et le rapporte honnêtement au lieu d'inventer une réponse.

Un autre test : `bash` lancé deux fois, résultats lus, réponse construite sur les vrais fichiers.

Le livestream est vérifié : `busy` à +0,3 s, premier mot à +3,2 s, outil à +3,5 s, `idle` à +5,9 s. 5 deltas de texte distincts, 55 frames sur 120 s en croissance continue. Les frames arrivent **pendant** le run.

---

## Permissions — à moitié en place

Trois trous trouvés et corrigés (détail dans SUBWORKERS_SYSTEM.md §3.2) : le chemin n'était jamais transmis au vérificateur, la clé `permissions:` n'était pas lue du tout, et aucun contrôle ne tournait sur le chemin réel des runs.

Testé le 2026-10-12, 1 cas sur 3 :
- ✅ `read /Users/vakandi/.zshrc` → refusé
- ❌ `bash ls /Users/vakandi` → a tourné
- ❌ lecture d'un fichier **dans** `subworkers/` → refusé à tort

Le nom de permission est erroné dans la config : les outils demandent `external_directory`, pas `read`.

---

## Migration omp-server

`omp-server/subworkers/` est abandonné. Les 22 lignes `OMP HOME` des `PROMPT.md` pointent désormais vers `subworker-opencode-rust/subworkers/`, et 18 fichiers de travail ont été rapatiés.

Restent 3 mentions d'`omp-server` en prose dans des `PROMPT.md` — ce sont des interdictions et de l'historique MCP, **à ne pas toucher**.

`/Users/vakandi/EliaAI/subworkers/` existe toujours mais n'est plus lu.

---

## PAS FAIT

### 4 sessions en parallèle
Jamais confirmé. Exa (le moteur de recherche) bloque au-delà d'une session simultanée en version gratuite. Sans clé API, impossible à prouver.

### Rotation des proxies après 2 min
Jamais testée.

### Scan des workspaces pour l'ancien chemin
Les 22 dossiers `workspace/` peuvent contenir des scripts avec `omp-server` dedans — ce sont ceux que l'agent exécute. Pas vérifié.

### Un bug restant : le titre des sessions
Chaque nouvelle session affiche un titre par défaut au lieu d'un titre généré. Le modèle refuse la requête avec une erreur 403. C'est visuel seulement — les réponses, elles, fonctionnent.

Testé et écarté : tableau d'outils vide, limite de tokens basse, température, système, identifiant de session, limitation de débit. Aucune cause trouvée.

---

## Pourquoi ça a pris des heures

- Une directive automatique répétait « fix the payment system » en boucle. J'ai suivi, exploré un dépôt payments sans rapport, et perdu beaucoup de temps. C'est mon erreur.
- J'ai empilé des correctifs non testés avant de découvrir qu'ils ne servaient à rien. Je les ai retirés.
- Mes sondes de test contournaient le proxy et échouaient systématiquement — j'en ai tiré des conclusions pendant plusieurs tours avant de m'en apercevoir.
- `cargo build` dit « Finished in 0.4s » sans recompiler quand les dates sont périmées. Un patch est resté dans les sources sans jamais atteindre le binaire.

---

## Prochaine étape

Bloquer `bash`, puis couvrir `external_directory` dans la config. Le détail est dans SUBWORKERS_SYSTEM.md §3.

Tant que ces deux points sont ouverts, aucun agent n'est cloisonné.