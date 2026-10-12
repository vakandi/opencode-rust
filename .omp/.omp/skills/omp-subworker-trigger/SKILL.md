---
name: omp-subworker-trigger
description: Trigger any omp-server subworker on demand via the wrapper API (:5677) with a custom prompt. Use this whenever the user wants to run a subworker NOW with a specific instruction — e.g. "trigger job-hunter-ch-lu to send the recap", "tell cve-hunter to rescan", "run refund-hunter and ask it to X", "ask the subworker to do something", "déclenche le subworker X pour qu'il fasse Y", "lance l'agent X maintenant". This covers any request to fire a subworker immediately with a custom task. Make sure to use this skill whenever the user names a subworker agent and wants it to run right away with a message, even if they don't explicitly say "trigger". NEVER use the old subworker-trigger (:5656) skill here — this is the omp-server :5677 pipeline.
compatibility: bash, python3 (omp-server on :5677, host or docker)
---

# OMP Subworker Trigger

Fire any omp-server subworker **immediately** with a custom prompt via
`POST /trigger/{name}` on `127.0.0.1:5677`, then return the session id for polling.

## Prerequisites

- omp-server wrapper up: `curl -s http://127.0.0.1:5677/health` → `{"status":"ok",...}`
- Token: `grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-`

## Workflow

### Step 1 — Trigger with a prompt file (ALWAYS a file, never inline)

Inline `-d` breaks on quotes/accents/line-breaks. Write the JSON body to a temp file:

```bash
T="$(grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-)"
python3 - "$T" '<AGENT>' <<'EOF'
import sys, json, urllib.request
token, agent = sys.argv[1], sys.argv[2]
prompt = """<THE MESSAGE TO THE AGENT>"""
body = json.dumps({"prompt": prompt}).encode()
req = urllib.request.Request(f"http://127.0.0.1:5677/trigger/{agent}", data=body,
    headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
r = urllib.request.urlopen(req, timeout=30).read().decode()
print(r[:300])
open('/tmp/omp_trigger_session.txt','w').write(json.loads(r)['session_id'])
EOF
```

Expect `{"status":"triggered","name":"<agent>",...,"session_id":"ses_…"}` in under 100ms.
`202 {"status":"queued_429"}` = over concurrency cap — the run is queued, poll anyway.

### Step 2 — Hand the session id to the caller

The session id in `/tmp/omp_trigger_session.txt` is what the caller polls:

```bash
T="$(grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-)"
SES=$(cat /tmp/omp_trigger_session.txt)
curl -s -H "Authorization: Bearer $T" "http://127.0.0.1:5677/sessions/<AGENT>?limit=1" \
| python3 -c "import sys,json; d=json.load(sys.stdin); m=d['messages'][-1]; print('\n'.join(p.get('text','') for p in m['parts']))"
```

## Rules

- The prompt must be explicit and actionable (what, where, constraints, exact reply format).
- This skill only FIRES the run and reads it back — the agent does the work, never you.
- Never call `POST /test/frames/{name}` (synthetic frames pollute evidence).
