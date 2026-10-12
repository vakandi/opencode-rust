/** Route handlers: /status, /trigger, /enable, /disable, PUT /status, /config/reload, /logs.
 * Each handler: (req, params) => Response. Auth applied in server.ts.
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import { admitRun } from "../admit.ts";
import { effectiveWorkspace, loadSubworkers, subworkersJsonPath } from "../auth.ts";
import { getEntry, isEnabled, setEnabled, reloadEntries, nextRunIso, schedulerRunning } from "../scheduler.ts";
import { readFrames } from "../store.ts";
import { statusPayload, statusItem } from "../statusView.ts";
import { broadcast, emitSubworkerStarted } from "../ws.ts";

export function json(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export function notFound(name: string): Response {
  return json({ detail: `unknown subworker ${name}` }, 404);
}

/** Write one entry's allowed fields back to subworkers.json so PUT /status
 *  edits (schedule, model, ...) survive reloads and restarts. Best-effort:
 *  the in-memory update above already applied, so a disk failure only logs. */
function persistEntry(name: string, updates: Record<string, unknown>): void {
  try {
    const path = subworkersJsonPath();
    const parsed = JSON.parse(readFileSync(path, "utf8"));
    const subs = Array.isArray(parsed?.subworkers) ? parsed.subworkers : null;
    if (!subs) return;
    const target = subs.find((e: unknown) => typeof e === "object" && e !== null && (e as Record<string, unknown>).name === name);
    if (!target) return;
    Object.assign(target, updates);
    parsed.last_modified = new Date().toISOString();
    writeFileSync(path, JSON.stringify(parsed, null, 2) + "\n");
  } catch (err) {
    console.error(`[status] persist failed name=${name}: ${String(err).slice(0, 200)}`);
  }
}

/** Best-effort: mirror a PUT /status model change into the persona file
 *  frontmatter (<agentDir>/agents/<name>.md `model:` line). Missing file,
 *  missing frontmatter, or missing model line = skip silently; a disk
 *  failure only logs, never fails the endpoint. */
function syncPersonaModel(name: string, model: string): void {
  try {
    const safe = basename(name).replace(/[^a-zA-Z0-9_-]/g, "");
    if (!safe) return;
    const home = homedir();
    const base = process.env.OMP_AGENT_DIR?.trim() || join(home, ".omp", "agent");
    const candidates = [
      join(base, "agents", `${safe}.md`),
      join(home, ".omp", "agent", "agents", `${safe}.md`),
      join(home, ".config", "omp", "agents", `${safe}.md`),
    ];
    const seen = new Set<string>();
    let path: string | undefined;
    for (const candidate of candidates) {
      if (seen.has(candidate)) continue;
      seen.add(candidate);
      if (existsSync(candidate)) {
        path = candidate;
        break;
      }
    }
    if (!path) return;
    const raw = readFileSync(path, "utf8");
    const match = raw.match(/^---\r?\n[\s\S]*?\r?\n---\r?\n/);
    if (!match) return;
    const block = match[0];
    if (!/^model:.*$/m.test(block)) return;
    const next = block.replace(/^model:.*$/m, () => `model: ${model}`);
    writeFileSync(path, next + raw.slice(block.length));
  } catch (err) {
    console.error(`[status] persona model sync failed name=${name}: ${String(err).slice(0, 200)}`);
  }
}

export async function handleGetStatus(): Promise<Response> {
  return json(statusPayload());
}

export async function handleGetStatusOne(name: string): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  const item = statusItem(name);
  return json({
    name: entry.name,
    enabled: isEnabled(name),
    running: item?.running ?? false,
    next_run: nextRunIso(name),
    schedule: entry.schedule ?? {},
    agent_id: entry.agent_id,
    timeout_minutes: entry.timeout_minutes ?? 30,
    max_retries: entry.max_retries ?? 2,
    model: entry.model ?? null,
    variant: entry.variant ?? null,
  });
}

export async function handleTrigger(name: string, req: Request): Promise<Response> {
  let body: { prompt?: string; model?: string; variant?: string } = {};
  try {
    if (req.headers.get("content-type")?.includes("json")) body = await req.json();
  } catch {
    body = {};
  }
  const result = await admitRun(name, { prompt: body.prompt, model: body.model, variant: body.variant });
  return json(result.body, result.httpStatus);
}

export async function handleEnable(name: string): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  if (isEnabled(name)) return json({ status: "already_enabled", name, enabled: true });
  setEnabled(name, true);
  emitSubworkerStarted(name);
  broadcast(statusPayload());
  return json({ status: "enabled", name, enabled: true });
}

export async function handleDisable(name: string): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  if (!isEnabled(name)) return json({ status: "already_disabled", name, enabled: false });
  setEnabled(name, false);
  broadcast(statusPayload());
  return json({ status: "disabled", name, enabled: false });
}

export async function handleUpdateStatus(name: string, req: Request): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  let body: Record<string, unknown> = {};
  try {
    body = await req.json();
  } catch {
    return json({ detail: "invalid JSON body" }, 422);
  }
  const allowed = ["agent_id", "model", "variant", "timeout_minutes", "max_retries", "schedule"];
  const updates: Record<string, unknown> = {};
  for (const key of allowed) {
    if (key in body) updates[key] = body[key];
  }
  if (Object.keys(updates).length === 0) return json({ detail: "No fields to update" }, 422);
  if ("schedule" in updates) {
    const sched = updates.schedule;
    if (!sched || typeof sched !== "object") return json({ detail: "invalid schedule" }, 422);
    const schedObj = sched as Record<string, unknown>;
    const t = schedObj.type;
    if (t === "every") {
      const every = Number(schedObj.every);
      if (!(every >= 1 && every <= 1440)) return json({ detail: "every must be 1..1440 minutes" }, 422);
    } else if (t !== "interval" && t !== "cron") {
      return json({ detail: `Invalid schedule type: ${String(t)}` }, 422);
    }
  }
  Object.assign(entry, updates);
  persistEntry(entry.name, updates);
  if (typeof updates.model === "string" && updates.model.length > 0) syncPersonaModel(entry.name, updates.model);
  broadcast(statusPayload());
  const item = statusItem(name);
  return json({
    name: entry.name,
    enabled: isEnabled(name),
    running: item?.running ?? false,
    next_run: nextRunIso(name),
    schedule: entry.schedule ?? {},
    agent_id: entry.agent_id,
    timeout_minutes: entry.timeout_minutes ?? 30,
    max_retries: entry.max_retries ?? 2,
    model: entry.model ?? null,
    variant: entry.variant ?? null,
  });
}

export async function handleConfigReload(): Promise<Response> {
  reloadEntries();
  const names = loadSubworkers().map((e) => e.name);
  return json({ status: "reloaded", added: [], removed: [], unchanged: names, total: names.length });
}

export async function handleLogs(name: string, url: URL): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  const linesParam = Number.parseInt(url.searchParams.get("lines") ?? "100", 10);
  const maxLines = Number.isFinite(linesParam) && linesParam > 0 ? Math.min(linesParam, 2000) : 100;
  const out: string[] = [];
  // frame-derived lines first (new stack source of truth)
  for (const f of readFrames(name).slice(-maxLines)) {
    out.push(`[${new Date(f.t).toISOString()}] ${f.field}: ${f.text.slice(0, 500)}`);
  }
  // mirror of live log files (old stack path, best-effort)
  let logFile: string | null = null;
  try {
    const workspace = effectiveWorkspace(entry);
    const logDir = join(workspace, "logs");
    if (existsSync(logDir)) {
      const files = readdirSync(logDir)
        .filter((f) => f.endsWith(".log"))
        .map((f) => join(logDir, f));
      if (files.length > 0) {
        logFile = files[files.length - 1];
        const all = readFileSync(logFile, "utf8").split("\n");
        for (const l of all.slice(-maxLines)) {
          if (l.trim()) out.push(l);
        }
      }
    }
  } catch {
    // best-effort only
  }
  const total = out.length;
  return json({ name, log_file: logFile, lines: out.slice(-maxLines), total_lines: total });
}

export function schedulerState(): { running: boolean } {
  return { running: schedulerRunning() };
}
