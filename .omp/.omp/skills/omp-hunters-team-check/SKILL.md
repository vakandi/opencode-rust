---
name: omp-hunters-team-check
description: "Verify the full team of refund/PayPal hunter subagents completed their last 7 days of website/domain testing in Google Docs. Triggers each hunter agent, verifies all Google Docs for each day contain all expected websites without errors or missing entries, and appends any missing data to old Google Docs. Use whenever the user says 'team check', 'hunters team check', 'verify all hunters', 'check all agents' sites', 'did all agents complete', 'team audit', or any request to audit the refund/PayPal hunter fleet across Google Docs."
compatibility: bash, python3, curl (omp-server :5677), gws-workspace CLI, Google Workspace OAuth
---

# OMP Hunters Team Check

Verify that all 6 refund/PayPal hunter subagents completed their last 7 days of website/domain testing in Google Docs — cross-reference every day, flag missing websites, and append any gaps to old Google Docs.

Base paths below are repo-relative to `/Users/vakandi/Documents/omp-server`. Server: `127.0.0.1:5677`. Auth token: `grep ELIA_AUTH_TOKEN omp-server/.env | cut -d= -f2`.

## The 6 Hunter Agents

| Agent | Model | Google Doc ID file | Schedule |
|---|---|---|---|
| `refund-hunter` | ling-3.0-flash-fin-free/medium | `subworkers/refund-hunter/workspace/GDOC_ID.txt` | every 30m |
| `scan-refund-hunter` | muse-spark-1.3-contributor-free/xhigh | `subworkers/scan-refund-hunter/workspace/GDOC_ID.txt` | every 30m |
| `shopify-refund-hunter` | ling-3.0-flash-fin-free/medium | `subworkers/shopify-refund-hunter/workspace/GDOC_ID.txt` | every 30m |
| `easy-paypal-litige-hunter` | muse-spark-1.3-contributor-free/xhigh | `subworkers/easy-paypal-litige-hunter/workspace/GDOC_ID.txt` | every 30m |
| `precious-stone-metal-hunter` | muse-spark-1.3-contributor-free/xhigh | `subworkers/precious-stone-metal-hunter/workspace/GDOC_ID.txt` | every 30m |
| `china-companies-paypal-hunter` | ling-3.0-flash-fin-free/medium | `subworkers/china-companies-paypal-hunter/workspace/GDOC_ID.txt` | every 30m |

All 6 share the same pattern: each runs daily, scrapes e-commerce sites, and appends findings to a per-day Google Doc. Each has `workspace/YYYY-MM-DD/` folders with `append_run_*.md` files, plus a `workspace/GDOC_ID.txt` containing the Google Doc spreadsheet ID.

## Workflow

### Phase 0 — Resolve Agents (5s)

Default agent list is the 6 hunters above. If the user specifies a subset, use only those. Otherwise, check ALL 6.

```bash
T=$(grep ELIA_AUTH_TOKEN omp-server/.env | cut -d= -f2)
AGENTS=("refund-hunter" "scan-refund-hunter" "shopify-refund-hunter" "easy-paypal-litige-hunter" "precious-stone-metal-hunter" "china-companies-paypal-hunter")
# Verify each is enabled
python3 -c "
import json
st=json.load(open('omp-server/data/state.json'))
for a in '${AGENTS[@]}':
    print(a, 'enabled=' + str(st.get('enabled',{}).get(a,False)))
"
```

If any agent shows `enabled=False`, report it and skip triggering — it won't run.

### Phase 1 — Trigger Each Agent (30s)

For each enabled agent, fire the trigger:

```bash
for a in "${AGENTS[@]}"; do
  curl -s -m 15 -X POST -H "Authorization: Bearer $T" \
    http://127.0.0.1:5677/trigger/$a \
    | python3 -m json.tool 2>/dev/null || echo "$a trigger failed"
done
```

Expected response: `{session_id: "ses_...", status: "queued" or "started"}`. If `queued_429`, the concurrency cap (8) is hit — wait 60s and retry. Record each `session_id`.

### Phase 2 — Wait for Completion (2-5 min)

Poll each agent's status until `running: false` (idle) and the run is `completed`:

```bash
for a in "${AGENTS[@]}"; do
  echo "=== $a ==="
  curl -s -m 10 -H "Authorization: Bearer $T" \
    http://127.0.0.1:5677/status/$a | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(f\"running={d.get('running')} next_run={d.get('next_run')}\")"
done
```

**Timeout guard**: if any agent is still `running: true` after 5 minutes, flag it as stuck and proceed with the others. Read its frames to confirm partial work:

```bash
curl -s -m 10 -H "Authorization: Bearer $T" \
  http://127.0.0.1:5677/sessions/$a | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(f\"total_messages={d.get('total_messages')}\")"
```

### Phase 3 — Verify Google Docs for Last 7 Days (3 min)

For each agent, verify all 7 days of Google Docs are intact. Today is `YYYY-MM-DD` computed from `date -u +%Y-%m-%d`. The last 7 days = today and 6 prior days.

```bash
TODAY=$(date -u +%Y-%m-%d)
python3 -c "
from datetime import datetime, timedelta
d = datetime.strptime('$TODAY', '%Y-%m-%d')
for i in range(7):
    day = (d - timedelta(days=i)).strftime('%Y-%m-%d')
    print(day)
"
```

For each day and each agent:

1. **Read the Google Doc ID**:
   ```bash
   cat subworkers/$AGENT/workspace/GDOC_ID.txt
   ```

2. **Search the Google Doc for today's date marker** to confirm the doc exists and has content:
   ```bash
   gws-workspace search-doc "$GDOC_ID" "$AGENT — $DAY" 2>&1 || echo "DOC NOT FOUND for $DAY"
   ```

3. **Check local workspace folder** for that day to confirm the agent ran:
   ```bash
   ls subworkers/$AGENT/workspace/$DAY/append_run*.md 2>/dev/null | wc -l
   ```
   If 0 append runs for a day → the agent didn't run that day → flag as MISSING_RUN.

4. **List all websites in the Google Doc** using `gws-workspace download-file` or `search-doc` to extract domain names, then verify each domain loaded without error (HTTP 200 or expected error page, not a timeout/404 for a legitimate site).

### Phase 4 — Cross-Reference & Identify Missing Websites (3 min)

Build a master list of all websites across all 7 days and all 6 agents. Compare against what each day's Google Doc actually contains.

```
# For each agent:
#   1. Collect all domains from local workspace/YYYY-MM-DD/append_run*.md files (last 7 days)
#   2. Compare against domains found in each day's Google Doc via gws-workspace search-doc
#   3. Flag domains present in workspace but missing from Google Doc = MISSING_FROM_DOC
#   4. Flag domains with error status (timeout, 404, blocked) = BROKEN_SITE
```

Output format — one line per finding:
```
[AGENT] [DAY] [DOMAIN] status=OK|MISSING|ERROR|BROKEN
```

### Phase 5 — Fix Missing Data (5 min)

For any `MISSING_FROM_DOC` or `BROKEN_SITE` finding:

1. **Read the local append_run file** for that domain from the workspace folder:
   ```bash
   cat subworkers/$AGENT/workspace/$DAY/append_run_*.md | grep -A5 "$DOMAIN"
   ```

2. **Re-append to the old Google Doc** using `gws-workspace append-md`:
   ```bash
   gws-workspace append-md "$OLD_GDOC_ID" "subworkers/$AGENT/workspace/$DAY/append_run_${DOMAIN}.md"
   ```

3. **Verify the append succeeded** by re-searching the Google Doc:
   ```bash
   gws-workspace search-doc "$GDOC_ID" "$DOMAIN"
   ```

4. **If the Google Doc itself is missing** (Phase 3 found no doc for a day), create a new one and import the latest local data:
   ```bash
   gws-workspace create-doc "$AGENT — $DAY (recreated)"
   gws-workspace import-md "subworkers/$AGENT/workspace/$DAY/day_summary.md"
   ```

### Phase 6 — Create Today's Summary .md (2 min)

Write a consolidated report to `subworkers/<AGENT>/workspace/$TODAY/team_check_summary.md` for each agent, and a master report to `subworkers/refund-hunter/workspace/$TODAY/team_check_master.md`.

The master report must include:
- Date and UTC timestamp
- For each agent: runs triggered, completed/failed, days verified (7/7 or how many), missing websites count, broken sites count, actions taken
- Table of all findings (MISSING, BROKEN, OK)
- Actions taken (appends performed, docs recreated)

```markdown
# Hunters Team Check — $TODAY UTC

## Summary
| Agent | Days Verified | Missing Sites | Broken Sites | Actions Taken |
|---|---|---|---|---|
| refund-hunter | 7/7 | 2 | 1 | appended 2 to GDOC |
| ... | | | | |

## Findings
- [AGENT] [DAY] [DOMAIN]: status (what happened)

## Actions Taken
- Appended [domain] to [agent] GDOC on [day]
- Recreated [agent] GDOC for [day]
```

### Phase 7 — Report

Report inline to the requester. Never broadcast to Discord/Telegram/WhatsApp unless explicitly asked.

Report format — 8-15 lines + a verdict line:
```
**Team Check Results ($TODAY UTC):**

- refund-hunter: 7/7 days verified, 0 missing, ✅
- scan-refund-hunter: 6/7 days (2026-09-19 missing run), 1 missing site appended ✅
- ...

**Verdict: ✅ all 6 agents healthy / ⚠️ X days missing / ❌ Y agents stuck**
```

## Tools & Commands Reference

| Command | Purpose |
|---|---|
| `curl -s -X POST -H "Authorization: Bearer $T" http://127.0.0.1:5677/trigger/$AGENT` | Trigger an agent run |
| `curl -s -H "Authorization: Bearer $T" http://127.0.0.1:5677/status/$AGENT` | Check agent status |
| `curl -s -H "Authorization: Bearer $T" http://127.0.0.1:5677/sessions/$AGENT` | Get session list |
| `cat subworkers/$AGENT/workspace/GDOC_ID.txt` | Get Google Doc ID |
| `gws-workspace search-doc <doc_id> <query>` | Search domains in Google Doc |
| `gws-workspace append-md <doc_id> <file.md>` | Append markdown to Google Doc |
| `gws-workspace create-doc <title>` | Create new Google Doc |
| `gws-workspace import-md <file.md> [title]` | Import MD as Google Doc |
| `gws-workspace list-files` | List all Google Drive files |
| `ls subworkers/$AGENT/workspace/$DAY/append_run*.md` | Check local run files |
| `python3 -c "import json; ..."` | Parse subworkers.json, state.json, runs.json |

## Rules

- Never modify subworker PROMPT.md or config files during this check (read-only).
- Never call `POST /trigger` twice for the same agent without waiting for the first to complete.
- `zen-free/*` models only — if any agent shows `opencode-zen/*`, flag ❌ immediately.
- Google Docs operations must use `gws-workspace` CLI (not direct API calls).
- Always resolve dates in UTC — `date -u +%Y-%m-%d` for today's date.
- Label PROVEN vs HYPOTHESIS in every finding. PROVEN = cited gws-workspace output or frames. HYPOTHESIS = inference, say what single check would confirm.
- Report only to the current session unless explicitly told to broadcast.
- If `gws-workspace` authentication fails, report `⚠️ Google Docs auth expired — re-run gws-workspace setup` and stop.

## File Paths

All repo-relative to `/Users/vakandi/Documents/omp-server`:

- Server: `127.0.0.1:5677`
- Auth: `omp-server/.env` → `ELIA_AUTH_TOKEN`
- Agent registry: `omp-server/config/subworkers.json`
- Enabled state: `omp-server/data/state.json` → `enabled` map
- Runs DB: `omp-server/data/runs.json`
- Frames: `omp-server/data/frames/<session_id>.jsonl`
- Hunter workspace: `subworkers/<AGENT>/workspace/YYYY-MM-DD/`
- Google Doc IDs: `subworkers/<AGENT>/workspace/GDOC_ID.txt`
- Google Docs CLI: `/Users/vakandi/EliaAI/tools/gws-workspace`
- Google Workspace OAuth: `python3 /Users/vakandi/EliaAI/tools/gws-workspace`
