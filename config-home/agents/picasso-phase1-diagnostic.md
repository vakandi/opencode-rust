<!-- on-demand include: loaded when Phase 1 (diagnostic) is triggered. Not an agent definition. -->

## 🔬 MANDATORY DIAGNOSTIC MODE (Before Any Refonte)

**RÈGLE ABSOLUE: Avant toute refonte de page, section ou composant, tu dois IMPÉRATIVEMENT passer par cette phase diagnostic.**

### Quand activer ce mode ?

Ce mode s'active quand l'utilisateur dit:
- "refais cette page"
- "améliore ma section"
- "refonte le composant X"
- "mon site/page ne convertit pas"
- "analyse et optimise [élément]"
- Toute demande de modification/amélioration d'existant

### PHASE 1: ANALYSE EXISTANTE (Always First)

Quand on te demande de refondre, commence PAR analyser le code existant:

1. **Analyse Structurelle**
   - Lis le code actuel completely
   - Comprends la hiérarchie HTML/JSX
   - Identifie les patterns utilisés

2. **Analyse UI/UX**
   - Logique des composants
   - États (hover, active, loading, error, empty)
   - Parcours utilisateur

3. **Analyse Responsivité**
   - Breakpoints existants
   - Mobile-first compliance
   - Touch targets

4. **Analyse Visuelle**
   - Hiérarchie typographique
   - Palette de couleurs
   - Espacement et rythme

5. **Analyse Conversion**
   - CTAs: clarté, placement, urgence
   - Trust signals: présents ou manquants ?
   - Points de friction potentiels

### PHASE 2: VÉRIFICATION RÉELLE (Si nécessaire)

**Si le code seul ne suffit pas pour un diagnostic fiable:**

1. **Lance le projet en local** si possible
2. **Ouvre la page concernée** dans le navigateur
3. **Vérifie le rendu réel** (pas seulement le code)
4. **Fais des captures d'écran** si possible
5. **Base ton audit sur: code + rendu réel**

### PHASE 2B: RECHERCHE 21ST.DEV OBLIGATOIRE

**TOUJOURS effectuer cette recherche APRES l'analyse existante mais AVANT l'implémentation:**

1. **Recherche systématique** sur 21st.dev avec les mots-clés correspondants
2. **Analyse les composants récents** de la semaine/mois
3. **Cross-reference** avec les composants community validés
4. **Évalue chaque composant** selon les critères fluidity/conversion/pertinence
5. **Sélectionne le meilleur** en justifiant ton choix
6. **Prévoy l'adaptation** au contexte du projet

**Cette phase n'est jamais optionnelle, même si tu penses "connaitre" les patterns.**

### PHASE 3: MINI DIAGNOSTIC (Avant Refonte)

**APRÈS l'analyse, mais AVANT d'implémenter, présente ce diagnostic:**

```markdown
## 🔍 MINI DIAGNOSTIC

### Problème Identifié
[Qu'est-ce qui ne fonctionne pas ou pourrait être amélioré ?]

### Hypothèse
[Pourquoi ce n'est pas optimal ? Quel est le root cause ?]

### Direction de Refonte
[Comment allons-nous résoudre ça ? Par où on commence ?]

### Ce qui sera changé
[Liste concrète des éléments qui seront modifiés]
```

### PHASE 4: QUESTIONS SÉLECTIVES (Only if truly needed)

**Reste autonome au maximum.**

Ne pose des questions QUE si:
- Informations critiques manquantes (accès, credentials)
- Choix stratégique qui impacte profondément le projet
- Direction бизнес (priorités бизнес не ясны)

**Types de questions À ÉVITER:**
- Questions rhétoriques
- Questions dont tu peux déduire la réponse
- Plus de 3 questions par séance

**Questions si vraiment nécessaires:** максимум 2-3 ciblées et actionnables.

### PHASE 5: VALIDATION APRÈS REFONTE

Après l'implémentation, explique:
- POURQUOI cette refonte fonctionne
- Ce qui a été corrigé par rapport au diagnostic
- Points à surveiller / tester

### RÈGLE D'HONNÊTETÉ

**TRÈS IMPORTANT:**
- Ne prétends pas avoir fait un audit visuel précis si tu n'as ni rendu réel ni capture
- Sois explicite: "D'après le code, je déduis que..." vs "J'ai vérifié et..."
- Si tu n'as pas pu lancer le projet, dis-le clairement

### CRITÈRE DE SUCCÈS

Le but n'est pas seulement de faire quelque chose de beau, mais:

| Objectif | Description |
|----------|-------------|
| **Beau** | Design premium, moderne, cohérent |
| **Clair** | Lisibilité, hiérarchie, navigation intuitive |
| **Moderne** | Patterns actuels, pas datés |
| **Cohérent** | Respect du brand, consistency |
| **Fluide** | Animations purposeful, transitions smooth |
| **Performant** | UX optimale, conversion améliorée |
| **Convertit** | Actions claires, trust signals, parcours sans friction |
