/** Scheduler — interval, every (clock-aligned), cron. Persists next_run in state.json.
 * Live semantics: interval {hours:[9..23], minute:0, days?}, every {every, hours?, days?},
 * cron {expression}. Triggers call the same admission path as POST /trigger.
 */
import type { Timeout } from "node:timers";
import { readState, writeState } from "./store.ts";
import { loadSubworkers, type SubworkerEntry } from "./auth.ts";
import { refreshModelsIfDue } from "./models.ts";

export type TriggerFn = (name: string) => Promise<{ admitted: boolean; reason?: string }>;

let triggerFn: TriggerFn | null = null;
let timer: Timeout | null = null;
const enabledOverrides = new Map<string, boolean>();
let entriesCache: SubworkerEntry[] = [];

export function setTriggerFn(fn: TriggerFn): void {
  triggerFn = fn;
}

export function listEntries(): SubworkerEntry[] {
  return entriesCache;
}

export function getEntry(name: string): SubworkerEntry | undefined {
  return entriesCache.find((e) => e.name === name);
}

export function isEnabled(name: string): boolean {
  return enabledOverrides.get(name) ?? getEntry(name)?.enabled !== false;
}

export function setEnabled(name: string, enabled: boolean): void {
  enabledOverrides.set(name, enabled);
  try {
    const s = readState();
    s.enabled = { ...(s.enabled ?? {}), [name]: enabled };
    writeState(s);
  } catch (err) {
    console.error(`[scheduler] enabled persist failed: ${String(err)}`);
  }
}

export function reloadEntries(): SubworkerEntry[] {
  entriesCache = loadSubworkers();
  // drop overrides for names that vanished
  const names = new Set(entriesCache.map((e) => e.name));
  for (const key of [...enabledOverrides.keys()]) {
    if (!names.has(key)) enabledOverrides.delete(key);
  }
  // restore persisted enable/disable overrides (survive restarts)
  try {
    const saved = readState().enabled ?? {};
    for (const [key, value] of Object.entries(saved)) {
      if (names.has(key)) enabledOverrides.set(key, value === true);
    }
  } catch (err) {
    console.error(`[scheduler] enabled restore failed: ${String(err)}`);
  }
  return entriesCache;
}

function scheduleOf(entry: SubworkerEntry): Record<string, unknown> | null {
  return entry.schedule ?? null;
}

function scheduleType(entry: SubworkerEntry): string | null {
  const s = scheduleOf(entry);
  const t = s?.type;
  return typeof t === "string" ? t : null;
}

function numList(value: unknown): number[] | null {
  if (!Array.isArray(value)) return null;
  const out = value.filter((v): v is number => typeof v === "number" && Number.isFinite(v));
  return out;
}

function dayMatch(days: number[] | null, date: Date): boolean {
  if (!days || days.length === 0) return true;
  // live: 0=Sun..6=Sat, JS getDay matches
  return days.includes(date.getDay());
}

/** Next run strictly after `from` for interval/every/cron. Null when unscheduled. */
export function nextRunAfter(entry: SubworkerEntry, from = new Date()): Date | null {
  const sched = scheduleOf(entry);
  if (!sched) return null;
  const type = scheduleType(entry);
  if (type === "interval") {
    const hours = numList(sched.hours) ?? [];
    const minute = typeof sched.minute === "number" ? sched.minute : 0;
    const days = numList(sched.days);
    // scan forward up to 8 days for the next matching slot
    const probe = new Date(from);
    probe.setSeconds(0, 0);
    for (let i = 0; i < 8 * 24 * 60; i++) {
      probe.setMinutes(probe.getMinutes() + 1);
      if (probe.getMinutes() !== minute) continue;
      if (!hours.includes(probe.getHours())) continue;
      if (!dayMatch(days, probe)) continue;
      return new Date(probe);
    }
    return null;
  }
  if (type === "every") {
    const every = typeof sched.every === "number" ? sched.every : 0;
    if (!(every >= 1 && every <= 1440)) return null;
    const hours = numList(sched.hours);
    const days = numList(sched.days);
    const probe = new Date(from);
    probe.setSeconds(0, 0);
    // clock-align: next minute boundary divisible by `every`
    for (let i = 0; i < 3 * 24 * 60; i++) {
      probe.setMinutes(probe.getMinutes() + 1);
      if ((probe.getHours() * 60 + probe.getMinutes()) % every !== 0) continue;
      if (hours && !hours.includes(probe.getHours())) continue;
      if (!dayMatch(days, probe)) continue;
      return new Date(probe);
    }
    return null;
  }
  if (type === "cron") {
    const expr = sched.expression;
    if (typeof expr !== "string") return null;
    return nextCronAfter(expr, from);
  }
  return null;
}
/** Minimal 5-field cron matcher (minute hour dom month dow), next occurrence after `from`. */
export function nextCronAfter(expr: string, from: Date): Date | null {
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) return null;
  const ranges: Array<[number, number]> = [[0, 59], [0, 23], [1, 31], [1, 12], [0, 7]];
  const fields: Array<Set<number> | undefined> = parts.map((p, i) => parseCronField(p, ranges[i][0], ranges[i][1]));
  const [minF, hourF, domF, monF, dowF] = fields;
  if (!minF || !hourF || !domF || !monF || !dowF) return null;
  const probe = new Date(from);
  probe.setSeconds(0, 0);
  for (let i = 0; i < 32 * 24 * 60; i++) {
    probe.setMinutes(probe.getMinutes() + 1);
    if (
      minF.has(probe.getMinutes()) &&
      hourF.has(probe.getHours()) &&
      domF.has(probe.getDate()) &&
      monF.has(probe.getMonth() + 1) &&
      dowMatch(dowF, probe.getDay())
    ) {
      return new Date(probe);
    }
  }
  return null;
}

function parseCronField(field: string, lo: number, hi: number): Set<number> | undefined {
  const out = new Set<number>();
  const addRange = (a: number, b: number, step: number): void => {
    for (let v = a; v <= b; v += step) out.add(v);
  };
  for (const chunk of field.split(",")) {
    const slash = chunk.split("/");
    const range = slash[0];
    const step = slash.length > 1 ? Number.parseInt(slash[1], 10) : 1;
    if (!Number.isFinite(step) || step < 1) return undefined;
    if (range === "*") {
      addRange(lo, hi, step);
    } else if (range.includes("-")) {
      const dash = range.split("-");
      const a = Number.parseInt(dash[0], 10);
      const b = Number.parseInt(dash[1], 10);
      if (!Number.isFinite(a) || !Number.isFinite(b)) return undefined;
      addRange(a, b, step);
    } else {
      const v = Number.parseInt(range, 10);
      if (!Number.isFinite(v)) return undefined;
      addRange(v, v, step);
    }
  }
  return out;
}

function dowMatch(set: Set<number>, dow: number): boolean {
  // cron 7 == Sunday
  return set.has(dow) || (dow === 0 && set.has(7));
}


export function nextRunIso(name: string): string | null {
  const state = readState();
  const cached = state.next_run[name];
  if (cached) return cached;
  const entry = getEntry(name);
  if (!entry) return null;
  const next = nextRunAfter(entry);
  return next ? next.toISOString() : null;
}

function refreshNextRuns(): void {
  const state = readState();
  let changed = false;
  for (const entry of entriesCache) {
    const next = nextRunAfter(entry);
    const iso = next ? next.toISOString() : "";
    if ((state.next_run[entry.name] ?? "") !== iso) {
      state.next_run[entry.name] = iso;
      changed = true;
    }
  }
  if (changed) writeState(state);
}

const firedSlots = new Map<string, number>();

async function tick(): Promise<void> {
  // Dynamic Zen roster refresh when due (30 min TTL inside). Awaited but
  // never breaks the tick: refreshModelsIfDue never throws, belt-and-braces catch.
  try {
    await refreshModelsIfDue();
  } catch (err) {
    console.error(`[scheduler] models refresh failed: ${String(err)}`);
  }
  if (!triggerFn) return;
  const now = new Date();
  refreshNextRuns();
  for (const entry of entriesCache) {
    if (!isEnabled(entry.name)) continue;
    const next = nextRunAfter(entry, new Date(now.getTime() - 60_000));
    if (!next) continue;
    // due when the next occurrence from 60s ago is now or past
    if (next.getTime() > now.getTime()) continue;
    // The 60s lookback above is wider than the tick interval, so a single due
    // slot stays "due" across consecutive ticks and would re-trigger each time.
    // Firing is keyed on the slot instant: one trigger per slot, ever.
    const slot = next.getTime();
    if (firedSlots.get(entry.name) === slot) continue;
    firedSlots.set(entry.name, slot);
    try {
      await triggerFn(entry.name);
    } catch (err) {
      firedSlots.delete(entry.name);
      console.error(`[scheduler] trigger failed name=${entry.name}: ${String(err)}`);
    }
  }
}

export function startScheduler(intervalMs = 30_000): void {
  reloadEntries();
  refreshNextRuns();
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
  timer = setInterval(() => {
    tick().catch((err) => console.error(`[scheduler] tick failed: ${String(err)}`));
  }, intervalMs);
  timer.unref?.();
}

export function schedulerRunning(): boolean {
  return timer !== null;
}

