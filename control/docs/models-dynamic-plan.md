# Dynamic Zen models plan — live roster, removals, NEW badges

Date: 20 Sep 2026. Status: PROPOSAL (not implemented).
Companion: `omp-server/SERVER.md` (route table), EliaTopBar `SubworkerManager.fetchModels` + `populateModelMenu`.

## 0. Findings (verified 2026-09-20, read-only)

- Canonical provider id is **`opencode-zen`** (`catalog/.../provider-ids.ts:56`,
  base `https://opencode.ai/zen/v1`), selector `opencode-zen/<id>`.
  `zen-free/…` exists nowhere in `models.json`. Our registry entries still say
  `opencode/muse-spark-…` — resolve falls back to default; entries should be
  migrated to `opencode-zen/…` (P0 with this plan).
- Live roster sources (both keyless): `GET https://opencode.ai/zen/v1/models`
  (74 bare ids, authoritative) and `https://catalog.stencil.so/models.json.zstd`
  (~187 KB, ETag/304, 104 opencode rows with `status:"deprecated"` on 33).
  Today: 2 ids live-but-unlisted (`jev-1.13*`), 32 stencil-only (mostly deprecated).
- Removal IS the deprecation signal (`dynamicModelsAuthoritative` + id pruning);
  static `models.json` carries no status flags. Nothing polls anywhere (2 h TTL
  lazy cache, per-`refresh()` only).
- Our `GET /models` parses `models.yml` (absent in containers → `total: 0`).
- TopBar: `fetchModels` at start/hover/reconnect only (no polling);
  `populateModelMenu` loop (`EliaTopBarApp.swift:2504-2507`) is the NEW-badge
  hook; persistence `model-selections.json` stores model WITHOUT variant.

## 1. omp-server changes

1. **New `models.ts` module** (replaces `models.yml` parsing in
   `routes/misc.ts:handleModels`): poll stencil (30 min, ETag/304) + confirm
   against zen endpoint; persist `{id, first_seen, status}` in
   `data/models.json` (gitignored); response extends `ModelOption` with
   `is_new: bool` (first_seen < 7 d), `deprecated: bool`, `first_seen: str`.
   Keep `id/name/provider/reasoning/variants/total` byte-identical otherwise.
   Deprecated/stale ids stay listed with `deprecated:true` for one cycle, then
   drop (TopBar keeps rendering them greyed while present).
2. **Scheduler hook**: refresh inside the existing 30 s tick when due
   (no new loop); failures silent + Discord only on flip events
   (added/removed), debounced.
3. **Registry migration**: rewrite entry `model` values
   `opencode/*` → `opencode-zen/*` (verify each id against the live roster
   first; unknown → leave + warn in logs, never break triggers).
4. **Contract test**: extend `contractCheck.ts` — `total > 0`, every option
   has `is_new/deprecated` booleans.

## 2. EliaTopBar changes

1. **Badge** (`EliaTopBarApp.swift`, `populateModelMenu` loop ~2504-2507):
   prefix `attributedTitle` with `NEW ` badge when `m.is_new`, grey out when
   `m.deprecated`. No custom view needed (same `NSMenuItem`).
2. **Refresh trigger**: refetch `/models` on every `status_update` that carries
   a bumped `models_version` (server includes it; see below) + on menu hover
   (existing). Fires `modelsLoadedNotification` → menu rebuilds live.
3. **Persist variant**: extend `model-selections.json` writer to
   `{agent: {model, variant}}` (reader stays backward-compatible with the
   bare-string shape); send both on select (server already accepts both).
4. **Server version counter**: `GET /status` + `status_update` gain
   `models_version: int` (incremented on every roster flip) so TopBar knows
   WHEN to refetch without polling `/models`.

## 3. Execution order + gates

1. Registry migration (dry-run list first) → 2. `models.ts` + route wiring →
   3. scheduler hook → 4. contractCheck asserts → 5. TopBar badge + version
   trigger → 6. variant persistence → 7. live soak: force-add/remove a canary
   id in staging? No — verify against REAL Zen flips over 48 h (log flips,
   confirm badge appears < 60 min after poll).
Gate each step: `bun -e` parse, contract script green, TopBar debug log shows
`models_version` bumps and badge rows.
