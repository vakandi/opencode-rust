/** omp-server wrapper entry — Bun.serve, Bearer auth, TopBar contract routes.
 * GET /health open; everything else Bearer-gated. Docs disabled (no /docs, /redoc, /openapi.json).
 */
import type { ServerWebSocket } from "bun";
import { authEnabled, checkHttpAuth, checkWsAuth, serverConfig } from "./auth.ts";
import { readState, sweepStalePrompts } from "./store.ts";
import { addClient, broadcast, removeClient, type WsData } from "./ws.ts";
import { admitRun } from "./admit.ts";
import { reloadEntries, setTriggerFn, startScheduler } from "./scheduler.ts";
import { initialStatusPayload, statusPayload } from "./statusView.ts";
import { startWatchdog } from "./watchdog.ts";
import { healthPayload } from "./healthIdentity.ts";
import {
  handleConfigReload,
  handleDisable,
  handleEnable,
  handleGetStatus,
  handleGetStatusOne,
  handleLogs,
  handleTrigger,
  handleUpdateStatus,
  json,
} from "./routes/status.ts";
import {
  handleContinueSession,
  handleGetSession,
  handleListSessions,
  handleSessionEvents,
  handleStopSession,
} from "./routes/sessions.ts";
import { handleServerCleanup, handleServerHealth, handleServerRestart } from "./routes/serverRoutes.ts";
import {
  handleGetMainAgent,
  handleModels,
  handleSetMainAgent,
  handleTestFrames,
  handleTunnelCheck,
  handleTunnelRemove,
  handleTunnelSetup,
  handleTunnelStatus,
  handleTunnelStop,
} from "./routes/misc.ts";

const bootAt = Date.now();
function unauthorized(): Response {
  return json({ detail: "Invalid or missing token" }, 401);
}

export async function route(req: Request, url: URL): Promise<Response> {
  const path = url.pathname;
  const method = req.method.toUpperCase();

  // open healthcheck — `repo` is the identity assert start.sh gates on
  if (method === "GET" && path === "/health") {
    return json(healthPayload(bootAt));
  }

  // docs disabled
  if (path === "/docs" || path === "/redoc" || path === "/openapi.json") {
    return json({ detail: "not found" }, 404);
  }

  // auth gate
  if (!checkHttpAuth(req, url).ok) return unauthorized();

  // status group
  if (method === "GET" && path === "/status") return handleGetStatus();
  const statusOne = path.match(/^\/status\/([^/]+)$/);
  if (statusOne) {
    if (method === "GET") return handleGetStatusOne(decodeURIComponent(statusOne[1]));
    if (method === "PUT") return handleUpdateStatus(decodeURIComponent(statusOne[1]), req);
  }
  const trigger = path.match(/^\/trigger\/([^/]+)$/);
  if (trigger && method === "POST") return handleTrigger(decodeURIComponent(trigger[1]), req);
  const enable = path.match(/^\/enable\/([^/]+)$/);
  if (enable && method === "POST") return handleEnable(decodeURIComponent(enable[1]));
  const disable = path.match(/^\/disable\/([^/]+)$/);
  if (disable && method === "POST") return handleDisable(decodeURIComponent(disable[1]));
  if (method === "POST" && path === "/config/reload") return handleConfigReload();
  const logs = path.match(/^\/logs\/([^/]+)$/);
  if (logs && method === "GET") return handleLogs(decodeURIComponent(logs[1]), url);

  // sessions group (order: list/continue/events before generic)
  const sessList = path.match(/^\/sessions\/([^/]+)\/list$/);
  if (sessList && method === "GET") return handleListSessions(decodeURIComponent(sessList[1]));
  const sessContinue = path.match(/^\/sessions\/([^/]+)\/([^/]+)\/continue$/);
  if (sessContinue && method === "POST") {
    return handleContinueSession(decodeURIComponent(sessContinue[1]), decodeURIComponent(sessContinue[2]), req);
  }
  const sessStop = path.match(/^\/sessions\/([^/]+)\/([^/]+)\/stop$/);
  if (sessStop && method === "POST") {
    return handleStopSession(decodeURIComponent(sessStop[1]), decodeURIComponent(sessStop[2]));
  }
  const sessEvents = path.match(/^\/sessions\/([^/]+)\/([^/]+)\/events$/);
  if (sessEvents && method === "GET") {
    return handleSessionEvents(decodeURIComponent(sessEvents[1]), decodeURIComponent(sessEvents[2]));
  }
  const sessGet = path.match(/^\/sessions\/([^/]+)$/);
  if (sessGet && method === "GET") return handleGetSession(decodeURIComponent(sessGet[1]), url);

  // server group
  if (method === "GET" && path === "/server/health") return handleServerHealth(url);
  if (method === "POST" && path === "/server/restart") return handleServerRestart();
  if (method === "POST" && path === "/server/cleanup") return handleServerCleanup(req);

  // tunnel group
  if (method === "GET" && path === "/tunnel/status") return handleTunnelStatus();
  if (method === "POST" && path === "/tunnel/check") return handleTunnelCheck(req);
  if (method === "POST" && path === "/tunnel/setup") return handleTunnelSetup(req);
  if (method === "POST" && path === "/tunnel/stop") return handleTunnelStop();
  if (method === "POST" && path === "/tunnel/remove") return handleTunnelRemove();

  // models + main-agent
  if (method === "GET" && path === "/models") return handleModels();
  if (method === "GET" && path === "/main-agent") return handleGetMainAgent();
  if (method === "POST" && path === "/main-agent") return handleSetMainAgent(req);

  // synthetic frame gate (auth-gated, test-scoped): emits text/reasoning/tool in-process
  const testFrames = path.match(/^\/test\/frames\/([^/]+)$/);
  if (testFrames && method === "POST") return handleTestFrames(decodeURIComponent(testFrames[1]));

  return json({ detail: "not found" }, 404);
}

function serveWs(ws: ServerWebSocket<WsData>): void {
  addClient(ws);
  ws.send(JSON.stringify({ event: "initial_status", ...initialStatusPayload(), scheduler_running: true }));
}

export function startServer(): void {
  const cfg = serverConfig();
  reloadEntries();
  setTriggerFn(async (name) => {
    const result = await admitRun(name, {});
    return { admitted: result.httpStatus === 200, reason: result.body.status };
  });
  startScheduler();
  startWatchdog();
  const state = readState();
  void state;
  const swept = sweepStalePrompts();
  console.log(`[omp-server] swept ${swept} stale prompts`);

  Bun.serve<WsData>({
    port: cfg.port,
    // HOST defaults to loopback; compose sets 0.0.0.0 so the mapped port is reachable in-container.
    hostname: process.env.HOST?.trim() || "127.0.0.1",
    fetch(req, server) {
      const url = new URL(req.url);
      if (url.pathname === "/ws") {
        if (!checkWsAuth(req, url)) return json({ detail: "Invalid or missing token" }, 401);
        const upgraded = server.upgrade(req, { data: { authed: true } });
        if (!upgraded) return json({ detail: "ws upgrade failed" }, 500);
        return new Response(null, { status: 101 });
      }
      return route(req, url);
    },
    websocket: {
      open: serveWs,
      message(ws, msg) {
        const text = typeof msg === "string" ? msg : "";
        if (text === "ping") { ws.send(JSON.stringify({ event: "pong" })); return; }
        try {
          const obj = JSON.parse(text);
          if (obj && (obj.type === "ping" || obj.event === "ping")) ws.send(JSON.stringify({ event: "pong" }));
        } catch { /* ignore non-JSON frames */ }
      },
      close(ws) {
        removeClient(ws);
      },
    },
  });
  console.log(`[omp-server] listening on http://${process.env.HOST?.trim() || "127.0.0.1"}:${cfg.port} auth=${authEnabled() ? "on" : "off"}`);
  // keep broadcasting fresh status every 30s so TopBar never goes stale
  const statusTimer = setInterval(() => broadcast({ event: "status_update", ...statusPayload(), opencode_health: "healthy", scheduler_running: true }), 30_000);
  statusTimer.unref?.();
}

if (import.meta.main) startServer();
