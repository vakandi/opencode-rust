/** W2.3 — control-plane identity health.
 *
 * The silent-green hazard: `start.sh` used to declare "already running and healthy"
 * off a hardcoded `"health_status":"healthy"` string, so a DIFFERENT server owning the
 * port (the live omp-server on :5677) satisfied the gate and start.sh exited 0 while
 * nothing of ours was running.
 *
 * Two contracts, both asserted against the real code — never a copy of it:
 *   1. GET /health carries `repo`, so a caller can prove WHICH checkout answered.
 *   2. start.sh's `server_healthy()` returns non-zero when `repo` != REPO_ID.
 *      The bash function is sliced out of start.sh itself and executed with real curl
 *      and real jq against a stub HTTP server, so this test breaks if start.sh
 *      regresses, if it stops reading `.repo`, or if it stops parsing the body.
 */
import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { route } from "../server.ts";
import { REPO_ID } from "../healthIdentity.ts";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const START_SH = join(REPO_ROOT, "start.sh");

test("GET /health carries the repo identity", async () => {
  const url = new URL("http://127.0.0.1:5680/health");
  const res = await route(new Request(url), url);

  expect(res.status).toBe(200);
  const body = (await res.json()) as Record<string, unknown>;

  // identity
  expect(body.repo).toBe(REPO_ID);
  expect(body.repo).toBe("subworker-opencode-rust");
  // the rest of the documented shape
  expect(body.status).toBe("ok");
  expect(typeof body.uptime_s).toBe("number");
  expect(typeof body.db_path).toBe("string");
  expect(typeof body.agents_count).toBe("number");
  // db_path must be the repo-local Rust data dir, never the live user one
  expect(body.db_path as string).toStartWith(join(REPO_ROOT, "server", "data"));
});

test("start.sh server_healthy() fails when repo != REPO_ID", async () => {
  const src = await Bun.file(START_SH).text();
  const fn = extractFunction(src, "server_healthy");
  expect(fn).not.toBeNull();

  const mk = (repo: string) =>
    Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch(req) {
        const path = new URL(req.url).pathname;
        if (path === "/health") {
          return Response.json({
            status: "ok",
            repo,
            db_path: join(REPO_ROOT, "server", "data"),
            agents_count: 0,
            uptime_s: 1,
          });
        }
        if (path === "/server/health") {
          if (req.headers.get("authorization") !== "Bearer test-token") {
            return Response.json({ detail: "Invalid or missing token" }, { status: 401 });
          }
          return Response.json({
            repo,
            health_status: "healthy",
            child: { status: "ok", version: "0.0.0-test" },
          });
        }
        return new Response("nope", { status: 404 });
      },
    });

  const good = mk(REPO_ID);
  const wrong = mk("some-other-server");
  try {
    // Sanity: the harness must ACCEPT the correct identity — otherwise the negative
    // assertion below would pass for the wrong reason.
    const okRun = await runServerHealthy(fn!, good.port, REPO_ID);
    expect(okRun.exitCode).toBe(0);

    // The contract: a foreign repo on the port must fail the gate.
    const badRun = await runServerHealthy(fn!, wrong.port, REPO_ID);
    expect(badRun.exitCode).not.toBe(0);
  } finally {
    good.stop(true);
    wrong.stop(true);
  }
});

/** Slice `name() { ... }` out of a shell script, honouring brace depth. */
function extractFunction(src: string, name: string): string | null {
  const lines = src.split("\n");
  const start = lines.findIndex((l) => new RegExp(`^${name}\\(\\)\\s*\\{`).test(l));
  if (start === -1) return null;
  let depth = 0;
  const out: string[] = [];
  for (let i = start; i < lines.length; i++) {
    out.push(lines[i]);
    for (const ch of lines[i]) {
      if (ch === "{") depth++;
      else if (ch === "}") depth--;
    }
    if (depth === 0) break;
  }
  return out.join("\n");
}

/** Run the extracted bash function with real curl + real jq against `port`. */
async function runServerHealthy(
  fn: string,
  port: number,
  expectedRepo: string,
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
  const dir = mkdtempSync(join(tmpdir(), "health-identity-"));
  try {
    const proc = Bun.spawn({
      cmd: ["bash", "-c", `${fn}\nserver_healthy`],
      env: {
        ...process.env,
        BASE: `http://127.0.0.1:${port}`,
        REPO_ID: expectedRepo,
        ELIA_AUTH_TOKEN: "test-token",
      },
      stdout: "pipe",
      stderr: "pipe",
      cwd: dir,
    });
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
    ]);
    return { exitCode, stdout, stderr };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}