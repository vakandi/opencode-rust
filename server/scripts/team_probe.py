#!/usr/bin/env python3
"""Drive omo's team tools over the real plugin-host JSON-RPC, bypassing the model.

Proves: team_create registers a member session, team_send_message writes a real inbox file.
"""
import json
import os
import subprocess
import sys
import glob
import threading
import http.server
import urllib.request
import socketserver

HOST = os.path.expanduser("~/Library/Caches/opencode/plugin-host.ts")
OMO = os.path.expanduser("~/.opencode/plugins/oh-my-opencode.js")
SERVER = sys.argv[1]
LEAD = sys.argv[2]
DIR = os.getcwd()

# Logging reverse proxy: the plugin's SDK client is pointed here instead of straight at the
# server, so the exact request/response bodies can be captured without touching Rust.
CAPTURE = []
UPSTREAM = SERVER


class Proxy(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _proxy(self, method):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        req = urllib.request.Request(
            UPSTREAM + self.path, data=body or None, method=method,
            headers={k: v for k, v in self.headers.items()
                     if k.lower() not in ("host", "content-length", "connection")},
        )
        try:
            with urllib.request.urlopen(req) as r:
                status, payload, hdrs = r.status, r.read(), dict(r.headers)
        except urllib.error.HTTPError as e:
            status, payload, hdrs = e.code, e.read(), dict(e.headers)
        CAPTURE.append({"method": method, "path": self.path,
                        "req": body.decode("utf-8", "replace")[:1200],
                        "status": status,
                        "res": payload.decode("utf-8", "replace")[:1200]})
        self.send_response(status)
        for k, v in hdrs.items():
            if k.lower() not in ("transfer-encoding", "content-length", "connection"):
                self.send_header(k, v)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        self._proxy("POST")

    def do_GET(self):
        self._proxy("GET")

    def do_PATCH(self):
        self._proxy("PATCH")

    def do_DELETE(self):
        self._proxy("DELETE")

    def log_message(self, *a):
        pass


class Threaded(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


proxy = Threaded(("127.0.0.1", 0), Proxy)
PROXY_PORT = proxy.server_address[1]
threading.Thread(target=proxy.serve_forever, daemon=True).start()
PLUGIN_SERVER = f"http://127.0.0.1:{PROXY_PORT}"
print("PROXY listening on", PLUGIN_SERVER, "->", UPSTREAM)


def frame(obj):
    body = json.dumps(obj).encode()
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def read_frame(f):
    """Read one Content-Length framed message. Unbuffered read(1) on the raw fd."""
    hdr = b""
    while not hdr.endswith(b"\r\n\r\n"):
        c = f.read(1)
        if not c:
            return None
        hdr += c
    n = 0
    for line in hdr.decode(errors="replace").split("\r\n"):
        if line.lower().startswith("content-length"):
            n = int(line.split(":")[1])
    buf = b""
    while len(buf) < n:
        chunk = f.read(n - len(buf))
        if not chunk:
            return None
        buf += chunk
    return json.loads(buf)


p = subprocess.Popen(
    ["bun", "run", HOST],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=None,
    bufsize=0,
)
f = p.stdout

p.stdin.write(frame({
    "jsonrpc": "2.0", "id": 1, "method": "initialize",
    "params": {"pluginPath": OMO, "context": {
        "worktree": DIR, "directory": DIR, "serverUrl": PLUGIN_SERVER}},
}))
p.stdin.flush()
resp = read_frame(f)
if not resp or "error" in (resp or {}):
    print("INITIALIZE FAILED:", json.dumps(resp)[:300])
    p.kill()
    sys.exit(1)
tools = resp["result"].get("tools", {})
print("STEP1 tools captured:", len(tools))
for t in ("team_create", "team_send_message"):
    print(f"  has {t}:", t in tools)
    if t in tools:
        print("   ", json.dumps(tools[t].get("parameters", {}))[:200])

spec = {
    "name": "probe-team",
    "members": [
        {"name": "alpha", "isLead": True, "category": "quick",
         "prompt": "Reply with ALPHA_READY only."},
        {"name": "beta", "category": "quick", "prompt": "Reply with BETA_READY only."},
    ],
}

p.stdin.write(frame({
    "jsonrpc": "2.0", "id": 2, "method": "tool.execute",
    "params": {"tool": "team_create", "context": {},
               "args": {"inline_spec": spec, "leadSessionId": LEAD}},
}))
p.stdin.flush()
resp = read_frame(f)
out = resp.get("result", {}).get("output") if resp else None
print("STEP2 team_create ->", json.dumps(out)[:500] if out else json.dumps(resp)[:400])

team_run_id = None
if isinstance(out, str):
    try:
        team_run_id = json.loads(out).get("teamRunId")
    except Exception:
        pass
elif isinstance(out, dict):
    team_run_id = out.get("teamRunId")
if not team_run_id and resp:
    txt = json.dumps(resp)
    for key in ("teamRunId", "team_run_id"):
        if key in txt:
            i = txt.index(key)
            print("  (sniff)", txt[i:i + 60])
print("  teamRunId =", team_run_id)

sessions = {}
if isinstance(out, str):
    try:
        d = json.loads(out)
        runtime = d.get("runtimeState") or d
        for m in runtime.get("members", []) or []:
            if isinstance(m, dict) and m.get("name"):
                sessions[m["name"]] = m.get("sessionId") or m.get("sessionID")
    except Exception:
        pass
print("  member sessions:", json.dumps(sessions))

if team_run_id:
    target = "beta"
    p.stdin.write(frame({
        "jsonrpc": "2.0", "id": 3, "method": "tool.execute",
        "params": {"tool": "team_send_message",
                   "context": {"sessionID": LEAD, "sessionId": LEAD,
                               "directory": DIR, "worktree": DIR},
                   "args": {"teamRunId": team_run_id, "to": target, "body": "PING"}},
    }))
    p.stdin.flush()
    resp = read_frame(f)
    out = resp.get("result", {}).get("output") if resp else None
    print("STEP3 team_send_message ->", json.dumps(out)[:300] if out else json.dumps(resp)[:300])

    inbox = os.path.expanduser(f"~/.omo/runtime/{team_run_id}/inboxes/{target}")
    print("  inbox dir:", inbox)
    print("  inbox files:", [os.path.basename(x) for x in glob.glob(inbox + "/*.json")][:5])
    print("  RESULT:", "INBOX_FILE_WRITTEN" if glob.glob(inbox + "/*.json") else "NO_INBOX_FILE")
    print("  TEAM_RUN_ID=" + team_run_id)
    for name, sid in sessions.items():
        print(f"  SESSION {name}={sid}")

p.kill()

print("\n=== WIRE CAPTURE (plugin SDK client -> Rust server) ===")
shown = 0
for i, c in enumerate(CAPTURE):
    if "session" not in c["path"].lower():
        continue
    shown += 1
    print(f"\n--- [{i}] {c['method']} {c['path']} -> {c['status']}")
    print("  REQ:", c["req"][:700])
    print("  RES:", c["res"][:700])
print(f"\n(session calls: {shown} / total captured: {len(CAPTURE)})")
