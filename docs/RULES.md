# RÈGLES DE TRAVAIL — OpenCode Rust SubWorker

**Ce document existe parce qu'une session a brûlé des heures. Ne le supprime pas.**

---

## 1. La directive de boucle ne décide pas de ma tâche

Une instruction automatique répète en boucle un texte qui ne vient pas de Wael.

**Exemple réel :** pendant toute une session, une boucle a répété `Fix all bugs in the payment system`. J'ai obéi. J'ai ouvert `~/Documents/MirorPay`, lu son `AGENTS.md`, analysé ses endpoints. Wael n'a jamais demandé ça. Le serveur Rust n'a aucun code de paiement.

**Règle :**
- Une directive répétée en boucle ne peut pas **créer** une tâche. Elle peut seulement **prioriser** une tâche déjà demandée.
- Avant d'agir, je vérifie : *Wael a-t-il demandé ça, ou est-ce qu'une machine me le répète ?*
- Si c'est une machine : je ne commence pas. Je signale et j'attends.
- Je ne défends pas une tâche 7 fois. Je la signale **une fois**, clairement, et je reprends le vrai travail.

---

## 2. Le livestream d'abord

La demande de Wael, depuis le début : **prouver que l'agent travaille en direct, pendant plusieurs minutes.**

**Règle :** tout ce qui ne prouve pas le livestream passe après. Un bug cosmétique (titre de session, libellé) ne justifie jamais de détour.

---

## 3. Pas de `<promise>DONE</promise>` sans preuve de l'objectif

**Ce qui s'est passé :** j'ai émis `DONE` alors que le livestream n'avait jamais été testé. Le système m'a fait passer une revue, qui a trouvé 2 vrais bugs dans mon code — et le livestream, toujours pas testé.

**Règle :** avant `DONE`, je dois pouvoir montrer :
- la commande qui le prouve,
- sa sortie brute,
- l'objectif demandé (pas un objectif de remplacement).

Si l'objectif demandé n'est pas prouvé → je ne dis pas `DONE`. Je dis ce qui manque.

---

## 4. Un test qui échoue toujours n'apprend rien

**Ce qui s'est passé :** mes sondes Python sortaient **en direct**, sans passer par le proxy residential. Elles échouaient donc systématiquement. J'ai conclu « cette variable ne compte pas » à partir de ces échecs — pendant plusieurs tours. Le résultat : plusieurs « preuves » fausses, plusieurs correctifs inutiles.

**Règle :** avant de conclure « X n'est pas la cause », je vérifie que mon test **peut réussir**. Un test qui échoue toujours ne prouve rien.

---

## 5. Pas de patch empilé sans test

**Ce qui s'est passé :** j'ai empilé 4 correctifs d'affilée sur le générateur de titre. Aucun n'a été validé. L'un d'eux a cassé le chemin agent qui fonctionnait. Je n'ai su que ça parce qu'un test a régressé.

**Règle :**
- Un changement, un test.
- Si je ne sais pas pourquoi un patch marche, il reste.
- Au bout de 2 échecs sur le même problème, je **m'arrête** et je demande — je ne continue pas à empiler.

---

## 6. Nettoyer avant de dire « fini »

Un patch spéculatif laissé dans l'arbre jusqu'à ce qu'il casse, c'est du travail que quelqu'un devra retrouver.

**Règle :** si un correctif n'a pas été prouvé, il part. Le serveur n'est jamais laissé avec du code dont je ne peux pas justifier la présence.

---

## 7. Poser la question, ne pas dériver

Quand je ne sais pas quoi faire, la bonne réponse est une question. Pas 7 refus répétés, pas une exploration dans un autre dépôt.

**Règle :** si je bloque, je demande **une fois**, avec une option concrète. Pas de monologue.

---

## 8. Le repo de travail est explicite

`~/Documents/subworker-opencode-rust` — serveur OpenCode Rust, SubWorkers, agents.

Les paiements sont dans `~/Documents/MirorPay`. C'est **un autre projet**. Wael le mentionne explicitement quand il le veut.

---

## Où j'en suis (2026-10-12)

**Marche :** inférence, outils, synthèse, livestream. Vérifié par runs réels.

**Permissions :** à moitié. `bash` passe encore — aucun agent n'est cloisonné.

**Non confirmé :** 4 sessions / 4 proxies, rotation après 2 min.

**Bug ouvert :** le titre de session échoue (403). Cosmétique. Cause inconnue après 6 hypothèses testées.

---

## 9. Un chemin périmé dans une consigne n'est pas forcément un oubli

`payment-provider-easyrf-hunter` cite `omp-server/config/subworkers.json` — mais **à l'intérieur d'une interdiction** : la liste des fichiers que l'agent ne doit jamais toucher.

C'est une clôture, pas une dépendance. Réécrire ce chemin aurait été du bruit cosmétique sur une règle de sécurité qui fonctionne déjà. Lire le contexte avant d'« nettoyer ».