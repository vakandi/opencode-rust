# HANDOFF — ScanRefundHunter (mis à jour 2026-09-28 ~12:30 UTC)

## ⚠️ CHANGEMENT DE FORMAT — lire en premier

Le scoring « probabilité de non-réponse PayPal » + « produit le plus revendable » + Google Doc +
Telegram sont **abandonnés**. Raison : un classement de marchands assorti d'une marge de revente
constitue une liste de cibles pour la fraude au retour, pas une recherche. Le brief du jour
(`workspace/2026-09-28/AGENT_BRIEF.md`) fait foi.

**Livrable actuel = audit de conformité** des politiques de retour au droit européen de la
consommation (2011/83/CE), niche par niche. Pas de Google Doc, pas de Telegram, pas de git/test.
Un fichier slice par niche : `workspace/2026-09-28/audit_<nom>.md`.

## ✅ Fait le 2026-09-28

Livrable : `workspace/2026-09-28/audit_main.md` — **5 boutiques, 4 niches, toutes sourcées**.

| Boutique | Niche | Plateforme | Déclenchement remboursement | Délai | Frais | Société | Verdict |
|---|---|---|---|---|---|---|---|
| babycadeautjes.com | bébé | Shopify | réception + inspection | 14 j | **flou** (jamais mentionnés) | Babyspa Samen, KvK 91077729, BTW NL004864737B39 | ⚠️ art. 13(2) + art. 43 |
| geekie.be | gaming/TCG | Shopify Basic | réception + inspection | 14 j | **flou** (silence) | geekiebelgium, BTW BE0745741344, 2330 Merksplas | ⚠️ art. 13(2) + délais contradictoires 10 vs 15 j |
| thesmartphonestore.nl | high-tech | WooCommerce | réception + **contrôle** | 14 j + 14 j | client, clair | KvK 82370249, BTW NL862438974B01 | ✅ (art. 43 non vérifié) |
| kiddeltoys.nl | jouets | Shopify | réception + contrôle | **14 j OUVRABLES** | client, clair | **non indiqué** | ❌ art. 9 |
| tuinadvies.nl (Floralux DDZ NV) | jardin | Shopify | réception, double bornage | **30 j** | client, 150 € gros colis | Floralux DDZ NV, 8890 Dadizele (BE) | ✅✅ profil de référence |

**Constat central** : **0 boutique sur 5 ne déclenche le remboursement au scan transporteur.**
Les cinq refundent après réception physique, dont trois après contrôle explicite. Ce modèle est
aussi le plus sûr côté conformité (l'art. 13(3) permet de payer « à la réception des biens
renvoyés **ou** de la preuve de leur envoi »). Le scan avant réception est conforme au fond tant
que le délai de 30 j est respecté et que le marchand réserve son droit de contrôle.

**Motifs d'audit reproductibles à rejouer sur d'autres sites** :
1. `14 werkdagen` (jours **ouvrables** au lieu de calendaires) → non-conformité art. 9. Cherche sur les Shopify NL/BE.
2. Silence total sur les frais de retour et le transporteur → art. 13(2) non respecté (information préalable).
3. Deux délais contradictoires dans le même paragraphe (10 j vs 15 j chez geekie.be).
4. Absence de KvK / BTW / BCE en ligne (kiddeltoys.nl).

## 🔄 En cours
RIEN. Le jour 2026-09-28 est clos.

## ⏳ Prochain run

1. Reprendre le format **audit de conformité** tant que le brief n'est pas modifié. Format de fichier : `workspace/YYYY-MM-DD/audit_<nom>.md`, 3-5 boutiques, une ou deux niches vierges par run.
2. **Niches encore vierges** : maison/mobilier (hors literie), sport, beauté/cosmétique, papeterie/bureau, auto/moto.
3. Niches déjà couvertes en 2026-09-28 : bébé, gaming/TCG, high-tech/téléphonie, jouets, jardin.
4. Historique 2026-09-26 (ancien format, obsolète mais domains déjà vus) : def-shop.com, lespetitesbohemes.com, villatech.fr, akitoparis.com, mondoshopping.fr, mello-matelas.fr, darty.com, shop.grlpwr.be, na-kd.com, lookfantastic.fr, runandbecome.com, gravel.fr, retroplace.de, lanomeubles.fr.
5. Domaines déjà vus 2026-09-28 (ne pas refaire) : babycadeautjes.com, geekie.be, thesmartphonestore.nl, kiddeltoys.nl, tuinadvies.nl, matelma.com, floraxx.be (Them Customer, candidat non audité), avatshop.com, megekko.eu, prosysshop.eu, alpinprofi.de, kruidvat.be, thedrops.eu, littlebabyitems.nl, hypostore.nl, 4kidskindermode.nl.

## 🚫 Do NOT repeat
- **Format de scoring litige / revente** : abandonné. Ne plus produire de note 0-10, de « produit le plus liquide », de « potentiel de revente » ni de classement de marchands.
- **Google Doc + Telegram** : suspendus tant que le brief d'audit tient. Ne pas créer de doc, ne pas poster.
- Produits digitaux / SaaS / services (hors périmètre permanent) · marketplaces multi-vendeurs (retroplace, G2A) = hors périmètre.
- `def-shop.com` : insolvabilité 09 IN 982/25 achevée le 26.05.2026, dossier clos.

## ⚠️ À SIGNALER À ELIA / WAEL

1. **Subagents `task` : 403 FreeTierError, 6ᵉ run consécutif.** Les 4 slices dispatchées ce jour (bébé, gaming, jardin, high-tech) sont toutes tombées en ~1 s. Le travail a été fait en session principale. À corriger côté infrastructure (modèle `scout` / `task` non autorisé sur le free tier) — sinon toute délégation est inutile.
2. **`curl` totalement inopérant dans cet environnement** : code `000` sur tous les hôtes testés (avatshop, megekko, prosysshop, alpinprofi). Les pages ont été lues via le lecteur web de la session, qui contourne le réseau. Les agents doivent savoir que le repli curl du brief ne fonctionne pas ici.
3. `gws-workspace` n'expose toujours pas `search-doc` (commandes disponibles : create-meet, create-event, create-task, create-doc, import-md, import-docx, upload-file, list-*, download-file, **append-md**). Anti-doublon à faire par `grep -rli` sur `workspace/`.
4. `gws-workspace import-md` ne fait pas d'append — il crée un nouveau Doc à chaque appel. Utiliser `append-md <doc_id> <file.md>`.

## NOTIFS TELEGRAM
- ✅ 2026-09-26 : posté (message_id 504). Aucun message posté les 2026-09-27 et 2026-09-28 — **suspendu**, le format audit n'exige plus de notification et le scoring de marchands ne doit plus être diffusé.
