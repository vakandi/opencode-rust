/** JSONL session/run store — every admission is fsynced before the HTTP response.
 * Paths: omp-server/data/runs.json, omp-server/data/frames/<sessionId>.jsonl, omp-server/data/state.json
 */
import { mkdirSync, existsSync, readFileSync, writeFileSync, appendFileSync, readdirSync, statSync, unlinkSync } from "node:fs";
import { join } from "node:path";

const DATA_DIR = new URL("./data/", import.meta.url).pathname;
const FRAMES_DIR = join(DATA_DIR, "frames");

export interface RunRecord {
  name: string;
  session_id: string;
  status: "running" | "queued_429" | "completed" | "failed" | "continued" | "cancelled";
  created_at: string;
  last_frame_at: string;
  started_at: string;
  model?: string | null;
  variant?: string | null;
  error?: string;
}

export interface Frame {
  t: number;
  field: "text" | "reasoning" | "tool";
  text: string;
  /** Bare tool name (no status glyph) — TopBar matches exact names. */
  tool?: string;
  /** Tool call arguments (object preferred) — feeds subagent panels. */
  input?: unknown;
  /** Truncated tool result text — feeds subagent panels. */
  output?: string;
}

function ensureDirs(): void {
  mkdirSync(FRAMES_DIR, { recursive: true });
}

export function runsPath(): string {
  return join(DATA_DIR, "runs.json");
}

export function readRuns(): RunRecord[] {
  ensureDirs();
  try {
    const raw = readFileSync(runsPath(), "utf8");
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

/** Persist full run list synchronously (fsync-equivalent for admission path). */
export function writeRuns(runs: RunRecord[]): void {
  ensureDirs();
  writeFileSync(runsPath(), JSON.stringify(runs, null, 2) + "\n");
}

export function upsertRun(record: RunRecord): void {
  const runs = readRuns();
  const idx = runs.findIndex((r) => r.session_id === record.session_id);
  if (idx >= 0) runs[idx] = record;
  else runs.push(record);
  writeRuns(runs);
}

export function getRun(sessionId: string): RunRecord | undefined {
  return readRuns().find((r) => r.session_id === sessionId);
}

export function runsFor(name: string): RunRecord[] {
  return readRuns().filter((r) => r.name === name);
}

export function runningSessionId(name: string): string | undefined {
  const rs = runsFor(name).filter((r) => r.status === "running" || r.status === "continued");
  rs.sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
  return rs[0]?.session_id;
}

export function lastSessionId(name: string): string | undefined {
  const rs = runsFor(name);
  rs.sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
  return rs[0]?.session_id;
}

export function activeRunCount(): number {
  return readRuns().filter((r) => r.status === "running" || r.status === "continued").length;
}

export function appendFrame(
  sessionId: string,
  field: Frame["field"],
  text: string,
  extra?: { tool?: string; input?: unknown; output?: string },
): void {
  ensureDirs();
  const frame: Frame = { t: Date.now(), field, text };
  if (extra?.tool !== undefined) frame.tool = extra.tool;
  if (extra?.input !== undefined) frame.input = extra.input;
  if (extra?.output !== undefined) frame.output = extra.output;
  appendFileSync(join(FRAMES_DIR, `${sessionId}.jsonl`), JSON.stringify(frame) + "\n");
}

export function readFrames(sessionId: string): Frame[] {
  const path = join(FRAMES_DIR, `${sessionId}.jsonl`);
  if (!existsSync(path)) return [];
  const out: Frame[] = [];
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      out.push(JSON.parse(trimmed) as Frame);
    } catch {
      // skip corrupt line, keep the rest
    }
  }
  return out;
}

export function frameFiles(): string[] {
  ensureDirs();
  try {
    return readdirSync(FRAMES_DIR).filter((f) => f.endsWith(".jsonl"));
  } catch {
    return [];
  }
}

// ── Persistent state (next_run per subworker, restart_count) ──

export interface PersistedState {
  next_run: Record<string, string>;
  restart_count: number;
  main_agent?: string;
  enabled?: Record<string, boolean>;
}

export function statePath(): string {
  return join(DATA_DIR, "state.json");
}
export function readState(): PersistedState {
  ensureDirs();
  try {
    const parsed = JSON.parse(readFileSync(statePath(), "utf8"));
    return {
      next_run: parsed.next_run ?? {},
      restart_count: parsed.restart_count ?? 0,
      main_agent: parsed.main_agent,
      enabled: parsed.enabled ?? {},
    };
  } catch {
    return { next_run: {}, restart_count: 0, enabled: {} };
  }
}

export function writeState(state: PersistedState): void {
  ensureDirs();
  writeFileSync(statePath(), JSON.stringify(state, null, 2) + "\n");
}

export function bumpRestartCount(): number {
  const s = readState();
  s.restart_count += 1;
  writeState(s);
  return s.restart_count;
}

export function snowflakeSessionId(): string {
  // ses_<snowflake-ish>: epoch-ms + random, monotonic enough for routing
  return `ses_${Date.now().toString(36)}${Math.floor(Math.random() * 0xffffff).toString(36).padStart(4, "0")}`;
}

export function nowIso(): string {
  return new Date().toISOString();
}

/** Delete stale prompt files older than maxAgeMs by mtime; returns deleted count. */
export function sweepStalePrompts(maxAgeMs = 3600000): number {
  const dir = join(DATA_DIR, "prompts");
  let files: string[];
  try {
    files = readdirSync(dir);
  } catch {
    return 0;
  }
  const cutoff = Date.now() - maxAgeMs;
  let deleted = 0;
  for (const f of files) {
    try {
      const p = join(dir, f);
      if (statSync(p).mtimeMs < cutoff) {
        unlinkSync(p);
        deleted++;
      }
    } catch {
      // best-effort per file
    }
  }
  return deleted;
}

/** Release a session's proxy-map entry (map hygiene for the watchdog path). */
export function releaseSessionProxy(sessionId: string): void {
  const path = join(DATA_DIR, "proxy-map.json");
  try {
    if (!existsSync(path)) return;
    const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return;
    const map = parsed as Record<string, unknown>;
    if (sessionId in map) {
      delete map[sessionId];
      writeFileSync(path, JSON.stringify(map, null, 2) + "\n");
    }
  } catch {
    // best-effort
  }
}
