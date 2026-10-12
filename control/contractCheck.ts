/** Read-only TopBar contract check — GETs only, never disturbs the hub server.
 * Usage: ELIA_AUTH_TOKEN=... bun omp-server/contractCheck.ts  (cwd: repo root)
 * Exit 0 + "CONTRACT-OK" on success, else exit 1 + "FAIL <route>: <detail>".
 */
const BASE = "http://127.0.0.1:5677";
const TOKEN = process.env.ELIA_AUTH_TOKEN ?? "";

function must(cond: unknown, msg: string): void {
  if (!cond) {
    console.error(`FAIL ${msg}`);
    process.exit(1);
  }
}

async function get(route: string, authed = true): Promise<unknown> {
  const headers: Record<string, string> = {};
  if (authed) headers["Authorization"] = `Bearer ${TOKEN}`;
  let res: Response;
  try {
    res = await fetch(BASE + route, { headers, signal: AbortSignal.timeout(10_000) });
  } catch (e) {
    console.error(`FAIL ${route}: fetch failed: ${e instanceof Error ? e.message : String(e)}`);
    process.exit(1);
  }
  must(res.ok, `${route}: HTTP ${res.status}`);
  try {
    return await res.json();
  } catch {
    console.error(`FAIL ${route}: non-JSON response`);
    process.exit(1);
  }
}

const isRec = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

// GET /health — open, {status:"ok"}
const health = (await get("/health", false)) as Record<string, unknown>;
must(health.status === "ok", `/health: expected status "ok", got ${JSON.stringify(health.status)}`);

// authed GET /server/health — health_status === "healthy"
const srv = (await get("/server/health")) as Record<string, unknown>;
must(srv.health_status === "healthy", `/server/health: expected health_status "healthy", got ${JSON.stringify(srv.health_status)}`);

// GET /status — total number, every subworker has boolean running
const status = (await get("/status")) as Record<string, unknown>;
must(typeof status.total === "number", `/status: expected numeric total, got ${JSON.stringify(status.total)}`);
must(Array.isArray(status.subworkers), `/status: expected subworkers array`);
for (const sw of status.subworkers as unknown[]) {
  must(isRec(sw) && typeof sw.running === "boolean", `/status: subworker missing boolean running: ${JSON.stringify(sw)?.slice(0, 120)}`);
}

// GET /status/elia — full StatusItem+ detail keys
const one = (await get("/status/elia")) as Record<string, unknown>;
for (const k of ["name", "enabled", "running", "schedule", "agent_id", "timeout_minutes", "max_retries", "model", "variant"]) {
  must(k in one, `/status/elia: missing key ${k}`);
}

// GET /models — {models array, total number}; total===models.length (shape only:
// network-flaky CI must not fail on roster size, only on contract shape).
const models = (await get("/models")) as Record<string, unknown>;
must(Array.isArray(models.models), `/models: expected models array`);
must(typeof models.total === "number", `/models: expected numeric total, got ${JSON.stringify(models.total)}`);
must(
  models.total === (models.models as unknown[]).length,
  `/models: total ${JSON.stringify(models.total)} !== models.length ${(models.models as unknown[]).length}`,
);
for (const m of models.models as unknown[]) {
  must(
    isRec(m) && typeof m.is_new === "boolean",
    `/models: option missing boolean is_new: ${JSON.stringify(m)?.slice(0, 120)}`,
  );
  must(
    isRec(m) && typeof m.deprecated === "boolean",
    `/models: option missing boolean deprecated: ${JSON.stringify(m)?.slice(0, 120)}`,
  );
}
console.log(`models total=${models.total}`);

// GET /main-agent — {name string}
const main = (await get("/main-agent")) as Record<string, unknown>;
must(typeof main.name === "string", `/main-agent: expected string name, got ${JSON.stringify(main.name)}`);

// GET /tunnel/status — {configured boolean}
const tunnel = (await get("/tunnel/status")) as Record<string, unknown>;
must(typeof tunnel.configured === "boolean", `/tunnel/status: expected boolean configured, got ${JSON.stringify(tunnel.configured)}`);

// GET /sessions/subworkers/list — {name, sessions array}
const list = (await get("/sessions/subworkers/list")) as Record<string, unknown>;
must(typeof list.name === "string", `/sessions/subworkers/list: expected string name`);
must(Array.isArray(list.sessions), `/sessions/subworkers/list: expected sessions array`);

// GET /logs/elia?lines=2 — {name, lines array, total_lines number}
const logs = (await get("/logs/elia?lines=2")) as Record<string, unknown>;
must(typeof logs.name === "string", `/logs/elia: expected string name`);
must(Array.isArray(logs.lines), `/logs/elia: expected lines array`);
must(typeof logs.total_lines === "number", `/logs/elia: expected numeric total_lines, got ${JSON.stringify(logs.total_lines)}`);

console.log("CONTRACT-OK");
