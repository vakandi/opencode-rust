/** TopBar-facing status view — single source for /status, /ws initial_status/status_update.
 * running comes from the run registry (status=="running"), never from schedule.
 */
import { runningSessionId } from "./store.ts";
import { getEntry, isEnabled, listEntries, nextRunIso, schedulerRunning } from "./scheduler.ts";
import { modelsVersion } from "./models.ts";

export interface StatusItem {
  name: string;
  enabled: boolean;
  running: boolean;
  next_run: string | null;
  schedule_type: string | null;
  schedule: Record<string, unknown> | null;
  model: string | null;
  variant: string | null;
}

export function statusItem(name: string): StatusItem | null {
  const entry = getEntry(name);
  if (!entry) return null;
  const schedule = entry.schedule ?? null;
  const rawType = schedule?.type;
  return {
    name: entry.name,
    enabled: isEnabled(entry.name),
    running: runningSessionId(entry.name) !== undefined,
    next_run: nextRunIso(entry.name),
    schedule_type: typeof rawType === "string" ? rawType : null,
    schedule,
    model: entry.model ?? null,
    variant: entry.variant ?? null,
  };
}
let cached: { scheduler_running: boolean; total: number; subworkers: StatusItem[]; models_version: number } | null = null;
let cachedAt = 0;

export function statusPayload(): { scheduler_running: boolean; total: number; subworkers: StatusItem[]; models_version: number } {
  const now = Date.now();
  if (cached && now - cachedAt < 1000) return cached;
  const subworkers: StatusItem[] = [];
  for (const entry of listEntries()) {
    const item = statusItem(entry.name);
    if (item) subworkers.push(item);
  }
  cached = { scheduler_running: schedulerRunning(), total: subworkers.length, subworkers, models_version: modelsVersion() };
  cachedAt = now;
  return cached;
}

export function initialStatusPayload(): Record<string, unknown> {
  const base = statusPayload();
  return { ...base, opencode_health: "healthy" };
}
