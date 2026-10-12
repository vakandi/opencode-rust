/** Dynamic Zen model roster — stencil poll (30 min, ETag/304) + zen endpoint confirm.
 * Snapshot: data/models.json {fetched_at, ids:{fullname:{first_seen,name,reasoning,variants,status,deprecated,stale}}},
 * ETag: data/models.etag. data/ is gitignored (root .gitignore: omp-server/data/).
 *
 * Roster ids are verbatim "<pid>/<mid>" (pid = stencil bucket: opencode | opencode-go;
 * go-only mids keep the opencode-go/ prefix, opencode bucket wins on collision);
 * opencode* buckets map to provider "opencode-zen". ids live in the zen endpoint
 * but missing from stencil (e.g. jev-1.13*) are listed as opencode/<mid> live rows.
 * Rows missing from BOTH sources stay listed deprecated:true for one cycle (stale),
 * then drop. New ids notify Discord (kind "models", debounced in notifier).
 *
 * Never throws to callers: refresh failures keep the last good snapshot;
 * getModels() returns [] only when nothing was ever fetched.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { notify } from "./notifier.ts";

export interface ModelOption {
  /** Verbatim roster id, "<pid>/<mid>" (pid: opencode | opencode-go). */
  id: string;
  name: string;
  /** "opencode-zen" for opencode* buckets. */
  provider: string;
  reasoning: boolean;
  variants: string[];
  /** first_seen < 7d. */
  is_new: boolean;
  /** stencil status deprecated, absent from zen confirm set, or stale (one-cycle grace). */
  deprecated: boolean;
  /** ISO timestamp. */
  first_seen: string;
}

interface SnapshotRow {
  first_seen: string;
  name: string;
  reasoning: boolean;
  variants: string[];
  status: string;
  deprecated: boolean;
  stale?: boolean;
}

interface Snapshot {
  fetched_at: string;
  ids: Record<string, SnapshotRow>;
  version: number;
}

/** Monotonic roster version for TopBar refetch triggers. Never throws. */
export function modelsVersion(): number {
  try {
    return loadSnapshot()?.version ?? 0;
  } catch {
    return 0;
  }
}

type StencilRow = Record<string, unknown>;

const DATA_DIR = new URL("./data/", import.meta.url).pathname;
const SNAPSHOT_PATH = join(DATA_DIR, "models.json");
const ETAG_PATH = join(DATA_DIR, "models.etag");

const STENCIL_URL = "https://catalog.stencil.so/models.json.zstd";
const ZEN_URL = "https://opencode.ai/zen/v1/models";
const TTL_MS = 30 * 60_000;
const NEW_MS = 7 * 24 * 3600_000;
const PROVIDER = "opencode-zen";

let cache: Snapshot | null = null;
let inflight: Promise<boolean> | null = null;

function loadSnapshot(): Snapshot | null {
  if (cache) return cache;
  try {
    if (!existsSync(SNAPSHOT_PATH)) return null;
    const parsed = JSON.parse(readFileSync(SNAPSHOT_PATH, "utf8")) as Snapshot;
    if (!parsed || typeof parsed.fetched_at !== "string" || !parsed.ids || typeof parsed.ids !== "object") return null;
    cache = parsed;
    return cache;
  } catch {
    return cache;
  }
}

function saveSnapshot(snap: Snapshot): void {
  try {
    mkdirSync(DATA_DIR, { recursive: true });
    writeFileSync(SNAPSHOT_PATH, JSON.stringify(snap, null, 2) + "\n");
    cache = snap;
  } catch (err) {
    console.error(`[models] snapshot write failed: ${String(err)}`);
  }
}

function readEtag(): string | null {
  try {
    return existsSync(ETAG_PATH) ? readFileSync(ETAG_PATH, "utf8").trim() || null : null;
  } catch {
    return null;
  }
}

function writeEtag(etag: string): void {
  try {
    mkdirSync(DATA_DIR, { recursive: true });
    writeFileSync(ETAG_PATH, etag + "\n");
  } catch (err) {
    console.error(`[models] etag write failed: ${String(err)}`);
  }
}

async function gunzipZstd(buf: ArrayBuffer): Promise<string> {
  // DecompressionStream("zstd") verified working under Bun; Bun.zstdDecompressSync fallback.
  try {
    const ds = new DecompressionStream("zstd");
    const stream = new Blob([buf]).stream().pipeThrough(ds);
    return await new Response(stream).text();
  } catch {
    return new TextDecoder().decode(Bun.zstdDecompressSync(Buffer.from(buf)));
  }
}

/** Bare mids from the authoritative zen endpoint, or null when unreachable. */
async function fetchZenIds(): Promise<Set<string> | null> {
  try {
    const res = await fetch(ZEN_URL, { signal: AbortSignal.timeout(15_000) });
    if (!res.ok) return null;
    const body = (await res.json()) as { data?: Array<{ id?: unknown }> };
    const data = Array.isArray(body) ? body : body.data;
    if (!Array.isArray(data)) return null;
    const ids = new Set<string>();
    for (const row of data) {
      if (row && typeof row.id === "string" && row.id) ids.add(row.id);
    }
    return ids;
  } catch (err) {
    console.error(`[models] zen confirm fetch failed: ${String(err)}`);
    return null;
  }
}

interface StencilBuckets {
  notModified: boolean;
  /** Per-bucket mid -> row (no cross-bucket dedupe; caller applies opencode-first). */
  buckets: Record<string, Record<string, StencilRow>>;
}

/** Stencil opencode* bucket rows, or null on failure. 304 yields empty buckets + notModified. */
async function fetchStencil(): Promise<StencilBuckets | null> {
  try {
    const headers: Record<string, string> = {};
    const etag = readEtag();
    if (etag) headers["If-None-Match"] = etag;
    const res = await fetch(STENCIL_URL, { headers, signal: AbortSignal.timeout(30_000) });
    if (res.status === 304) return { notModified: true, buckets: {} };
    if (!res.ok) {
      console.error(`[models] stencil fetch HTTP ${res.status}`);
      return null;
    }
    const fresh = res.headers.get("etag");
    if (fresh) writeEtag(fresh);
    const text = await gunzipZstd(await res.arrayBuffer());
    const catalog = JSON.parse(text) as Record<string, { models?: Record<string, StencilRow> }>;
    const buckets: Record<string, Record<string, StencilRow>> = {};
    for (const bucket of ["opencode", "opencode-go"]) {
      const models = catalog[bucket]?.models;
      if (models && typeof models === "object") buckets[bucket] = models;
    }
    return { notModified: false, buckets };
  } catch (err) {
    console.error(`[models] stencil fetch failed: ${String(err)}`);
    return null;
  }
}

/** Synchronous read-only view of the last good snapshot. Never throws. */
export function getModels(): ModelOption[] {
  try {
    const snap = loadSnapshot();
    if (!snap) return [];
    const opts: ModelOption[] = Object.entries(snap.ids).map(([id, row]) => ({
      id,
      name: row.name || id,
      provider: PROVIDER,
      reasoning: row.reasoning === true,
      variants: Array.isArray(row.variants) ? row.variants : [],
      is_new: row.deprecated !== true && Date.now() - Date.parse(row.first_seen) < NEW_MS,
      deprecated: row.deprecated === true,
      first_seen: row.first_seen,
    }));
    opts.sort((a, b) => (a.id.toLowerCase() < b.id.toLowerCase() ? -1 : 1));
    return opts;
  } catch {
    return [];
  }
}

/**
 * Refresh when due (no snapshot, or fetched_at older than 30 min).
 * Returns true when a refresh ran. Never throws; failures keep last good.
 */
export function refreshModelsIfDue(): Promise<boolean> {
  try {
    const snap = loadSnapshot();
    if (snap && Date.now() - Date.parse(snap.fetched_at) < TTL_MS) return Promise.resolve(false);
    if (inflight) return inflight;
    inflight = doRefresh(snap).finally(() => {
      inflight = null;
    });
    return inflight;
  } catch {
    return Promise.resolve(false);
  }
}

async function doRefresh(priorSnap: Snapshot | null): Promise<boolean> {
  try {
    const prior: Record<string, SnapshotRow> = priorSnap?.ids ?? {};
    const hadPrior = priorSnap !== null;
    const stencil = await fetchStencil();
    if (!stencil) return false;
    const zen = await fetchZenIds();
    const nowIso = new Date().toISOString();
    const next: Record<string, SnapshotRow> = {};

    if (stencil.notModified) {
      // Roster unchanged upstream: carry prior rows, drop already-stale ones,
      // re-apply zen-absence against the fresh confirm set.
      for (const [id, row] of Object.entries(prior)) {
        if (row.stale === true) continue;
        const mid = id.includes("/") ? id.slice(id.indexOf("/") + 1) : id;
        next[id] = { ...row, deprecated: row.status === "deprecated" || (zen !== null && !zen.has(mid)) };
      }
    } else {
      const opencode = stencil.buckets["opencode"] ?? {};
      const go = stencil.buckets["opencode-go"] ?? {};
      const emit = (pid: string, mid: string, row: StencilRow | null) => {
        const id = `${pid}/${mid}`;
        const old = prior[id];
        const status = row && typeof row["status"] === "string" ? (row["status"] as string) : "live";
        let variants: string[] = [];
        const opts = row?.["reasoning_options"];
        if (Array.isArray(opts)) {
          const effort = opts.find(
            (o): o is { values: unknown } =>
              !!o && typeof o === "object" && (o as Record<string, unknown>)["type"] === "effort",
          );
          if (effort && Array.isArray(effort.values)) {
            variants = (effort.values as unknown[]).filter((v): v is string => typeof v === "string");
          }
        }
        next[id] = {
          first_seen: old?.first_seen ?? nowIso,
          name: (row?.["name"] as string) || mid,
          reasoning: row?.["reasoning"] === true,
          variants,
          status,
          deprecated: status === "deprecated" || (zen !== null && !zen.has(mid)),
        };
      };
      for (const [mid, row] of Object.entries(opencode)) emit("opencode", mid, row);
      for (const [mid, row] of Object.entries(go)) {
        if (!(mid in opencode)) emit("opencode-go", mid, row);
      }
      if (zen) {
        // Live-but-unlisted ids (e.g. jev-1.13*): opencode/<mid> live rows.
        for (const mid of zen) {
          if (!(mid in opencode) && !(mid in go)) emit("opencode", mid, null);
        }
      }
      // Stale grace: prior ids absent from the new roster stay one cycle, then drop.
      for (const [id, row] of Object.entries(prior)) {
        if (!(id in next)) {
          if (row.stale !== true) next[id] = { ...row, stale: true, deprecated: true };
        }
      }
    }

    const added = hadPrior ? Object.keys(next).filter((id) => !(id in prior)) : [];
    const wentStale = hadPrior
      ? Object.keys(next).filter((id) => prior[id]?.stale !== true && next[id].stale === true)
      : [];
    const version = (priorSnap?.version ?? 0) + (hadPrior && added.length + wentStale.length > 0 ? 1 : 0);
    saveSnapshot({ fetched_at: nowIso, ids: next, version });

    if (hadPrior) {
      for (const id of added) {
        await notify("models", "models", `new model listed: ${id}`).catch(() => {});
      }
      for (const id of wentStale) {
        await notify("models", "models", `removed from live roster (greyed one cycle): ${id}`).catch(() => {});
      }
    }
    return true;
  } catch (err) {
    console.error(`[models] refresh failed: ${String(err)}`);
    return false;
  }
}
