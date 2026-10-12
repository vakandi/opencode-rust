<!-- This file is NOT an OMP agent definition — it has no name/description frontmatter so OMP discovery skips it. It is a shared rules template included by the refund-hunter subagents. -->

# Shared Refund-Hunter Rules — Canonical Template

> This file is INCLUDED by all refund-hunter agents via a pointer. Do not duplicate its content in individual agent files — edit HERE and all agents inherit the change.

---

## 📋 LEXIQUE VOCABULAIRE

### Général :
- **CNI sur fond** = un fichier Photoshop d'une carte d'identité modifiable avec un fond
- **JDD** = Justificatif de domicile style facture d'opérateurs, électricité
- **PDD** = Preuve de dépôt, un document fourni lorsque vous déposez un colis en point relais
- **AH** = Un document à remplir pour attester sur l'honneur quelque chose (que du vent), souvent la non-réception d'un colis
- **COINFLIP** = 50/50 (aléatoire)
- **PR/POLICE REPORT** = Un document lorsque vous portez plainte au commissariat (la plainte en gros). Certains sites le demandent, le type de plainte effectué est pour usurpation d'identité/vol simple
- **SPOOF** : Technique permettant d'usurper le numéro de téléphone de la police/autres. Parfait lorsque la boutique appelle le numéro du commissariat
- **SCAN** = Attribut pour la modification du statut d'un colis via insider
- **MANUEL** = Attribut pour la modification du statut d'un colis via un bordereau modifié

### Techniques lorsque vous enclenchez un retour de commande :
- **FTID** = Votre colis est livré chez le destinataire
- **LIT TRANSIT** = Votre colis a été pris en charge dans un relais et ne donne plus de signe de vie dans le transit
- **LIT** = Votre colis a été pris en charge par le point relais et ne donne plus de signe de vie
- **FTIDNA** = Votre colis a été livré dans un autre pays

### Techniques lors de la réception du colis :
- **DNA** = Déclaré ne pas avoir reçu le colis
- **EB** = Déclaré avoir reçu un colis vide
- **PEB** = Déclaré avoir reçu une partie des articles
- **REROUTE** = Votre colis est livré une deuxième fois ou reçoit une nouvelle étape de transit après avoir été livré (aucun sens) // sert de DNA avancé
- **RTS** : Colis retourné à l'expéditeur
- **Livré** : Ajoute la mention livré au suivi du colis
- **LOST** : Mention perte du colis
- **DOMMAGED** : Mention colis endommagé et donc retourner à l'expéditeur
- **INVEST CLAIM/IC** = Scan spécial de UPS déclarant avoir perdu le colis (enquête + fin d'enquête)
- **IC SECURE** = Scan invest claim souvent plus cher sur des suivis protégés, des IC basique

---

## 📝 Recherche STRICTEMENT légitime et en lecture seule

Tu documentes les politiques publiques et les retours d'expérience clients. Tu ne recommandes, ne prépares et ne facilites AUCUNE fausse déclaration (faux non-reçu, faux colis vide, faux dommage), AUCUN faux document (fausse pièce d'identité, faux justificatif, faux dépôt, fausse attestation, faux dépôt de plainte), et AUCUNE manipulation de suivi ou usurpation (spoof, scan insider, bordereau modifié, reroute abusif, invest claim frauduleux).

---

## 📦 Gestion du Google Doc quotidien

### Convention UNIQUE : `append-md` (pas `import-md`)

**Règle absolue — `import-md` crée un NOUVEAU doc à chaque appel. JAMAIS pour les appends. Utiliser `append-md <doc_id> <file>` avec l'ID du doc existant.**

Workflow par run :
1. Date du run : `TODAY=$(date +%F)` — JAMAIS depuis le handoff ou un fichier.
2. Lire `workspace/GDOC_ID.txt` et `workspace/GDOC_DATE.txt`.
3. Si `GDOC_DATE.txt` == `$TODAY` ET `GDOC_ID.txt` non vide → **append** avec `gws-workspace append-md "$GDOC_ID" "fichier.md"`.
4. Si `GDOC_DATE.txt` != `$TODAY` ou fichiers absents → **créer** avec `gws-workspace create-doc "Rapport $TODAY"`, sauvegarder l'ID et la date dans les fichiers.
5. ⚠️ `import-md` est INTERDIT pour les appends — il crée un nouveau doc à chaque fois et casse le suivi quotidien.
6. **Nom du doc** : `Rapport YYYY-MM-DD` UNIQUEMENT. Pas de "Run N", "Quotidien", "Complet", heure.

### Handoff obligatoire

Lire `workspace/HANDOFF_NEXT_SESSION.md` au début de chaque run, écrire à la fin. C'est le système de mémoire entre les runs.

---

## 📱 Règle UN SEUL message Telegram par jour

**Six agents chasseurs partagent le même topic Telegram. UN SEUL message par agent par jour, jamais plus.**

Workflow :
1. À CHAQUE run, appeler `mcp-cli call refund-telegram-agents today_sent '{}'` et chercher TON titre canonique exact.
2. Si déjà posté → **NE PAS renvoyer**. Appender les découvertes dans le Doc.
3. Si absent → envoyer UNE fois via `share_for_community` (lien + résumé + produits).
4. **Message initial = MINIMAL** : juste le titre + lien du doc. AUCUN chiffre, AUCUN compte de sites/boutiques/sociétés.
5. Titre canonique fixe : `NomAgent — Rapport $TODAY` (identique au caractère près entre check et envoi — les emoji vivent dans le corps, jamais la ligne 1).
6. **Messages par site/boutique/société INTERDITS** — ton travail c'est appender dans le Doc.

### Coordination entre les 6 agents chasseurs

Les 6 agents (`refund-hunter`, `shopify-refund-hunter`, `scan-refund-hunter`, `easy-paypal-litige-hunter`, `precious-stone-metal-hunter`, `china-companies-paypal-hunter`) postent TOUS dans le même topic Telegram. Pour éviter les collisions :
- Chaque agent utilise un **titre canonique unique** par jour (ex: `RefundHunter — Rapport 2026-09-23`, `ShopifyRefundHunter — Rapport 2026-09-23`, etc.).
- Le `today_sent` check vérifie TON titre exact, pas la date seule.
- Si un autre agent a déjà posté, NE PAS renvoyer le tien — attendre le lendemain.
- **Jamais deux agents ne postent en même temps dans un run. Si un run se termine et le Telegram n'est pas encore posté, le poster avant de considérer le run complet.**
- Le lien Telegram du jour reste celui du premier run de la journée (Run 1). Les runs suivants appendent dans ce même doc.

---

## 🚫 Anti-Patterns communs

- ❌ Ne pas se contenter d'un simple "le site existe" — toujours creuser CGV/retours/transporteur.
- ❌ Ne pas append de données brutes non filtrées — chaque entrée doit avoir les critères vérifiés.
- ❌ Ne jamais modifier le code des projets EliaAI.
- ❌ Ne pas dupliquer des sites/déjà analysés (consulte l'historique dans le workspace et le Google Doc).
- ❌ Ne PAS arrêter un run après 3 sites — minimum 5 sites analysés en profondeur par run (sauf indication contraire dans l'agent spécifique).
- ❌ Réutiliser l'ID de `GDOC_ID.txt` sans vérifier la date du jour dans `GDOC_DATE.txt` — Toujours `TODAY=$(date +%F)` + comparaison avec `GDOC_DATE.txt` avant chaque append et chaque envoi Telegram.
- ❌ Utiliser `import-md` pour les appends — crée un NOUVEAU doc à chaque fois. Utiliser `append-md <doc_id> <file>`.
- ❌ Nommer le doc autrement que `NomAgent — Rapport YYYY-MM-DD`.
- ❌ Oublier d'envoyer le lien du doc sur Telegram — la Phase 5.5 est OBLIGATOIRE (`today_sent` puis `share_for_community` si absent).
- ❌ Envoyer un message par site/boutique/société — INTERDIT. UN SEUL message/jour.
- ❌ Improviser des emoji dans la ligne titre Telegram — titre canonique fixe, toujours.
- ❌ Recommander ou décrire comment commettre une fraude au remboursement — STRICTEMENT INTERDIT, même si demandé.

---

## ⚖️ Legalité absolue

Aucune recommandation de méthode frauduleuse, aucun faux document, aucune fausse réclamation, aucune manipulation de suivi. Documenter uniquement le processus de retour officiel et les expériences clients publiques.

---

## 📊 Schéma de output (`output` frontmatter)

Chaque agent hunter produit des résultats selon ce schéma commun :

```json
{
  "agent": "nom-de-l-agent",
  "date": "YYYY-MM-DD",
  "results": [
    {
      "shop": "nom de la boutique/site",
      "url": "https://...",
      "platform": "Shopify|WooCommerce|PrestaShop|Magento|custom",
      "score": 0-10,
      "verdict": "🟢 CIBLE|🟡 PARTIEL|🔴 NON",
      "evidence": ["URL1", "extrait2", ...],
      "refundPolicy": "résumé de la politique",
      "returnShipping": "détails du transporteur",
      "resalePotential": "évaluation du potentiel de revente",
      "companyCodes": {"siret": "...", "status": "..."},
      "productCheck": "résultat de la vérification des produits"
    }
  ]
}
```

Chaque champ `shop`, `score`, `verdict`, `evidence` DOIT être présent pour chaque résultat. Les agents spécialisés (scan-refund-hunter, easy-paypal-litige-hunter) ont des champs supplémentaires (`companyCode`, `litigePattern`) ; les agents de niche ajoutent leurs propres champs spécifiques.

---

*Document partagé — toute modification ici bénéficie à TOUS les agents chasseurs. Ne pas dupliquer ailleurs.*
