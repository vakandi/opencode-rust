# PLAN — subworker-opencode-rust

> **Provenance:** produced by the `plan` agent (session `ses_eee723736ffep78DEtAk447Jy2`) from a
> 3-round adversarial hyperplan pass by 5 category members (unspecified-low, unspecified-high,
> ultrabrain, artistry, deep) over team `895ddeea-dffc-484e-8c12-7ea55c6bb468`.
> Facts were verified against disk by the lead and independently re-verified by members.
> **Do not re-derive. Execute wave by wave.**

Shorthand: `NEW=/Users/vakandi/Documents/subworker-opencode-rust`
Read-only inputs: `SRC_RUST=~/Documents/opencode-server-rust`, `SRC_OMP=~/Documents/omp-server`,
`SRC_ELIA=~/EliaAI/subworkers`

---

## BINDING DECISION (Wael)

The opencode rust server's **database lives INSIDE its repo at a relative path**. It does NOT live
in `~/Library/Application Support`. Every DB site resolves through the new crate
`server/crates/opencode-paths`, driven by `OPENCODE_PATHS_ROOT`.

Two suffixes are **explicit named constants** and must never be normalised together:

- provider auth = `<root>/opencode/data/auth.json`
- DB = `<root>/opencode/opencode.db`

Setting `$HOME` is **forbidden** — it breaks persona loading and `.omp/skills`.
Do **not** add a repo-root `AGENTS.md`; `instruction.rs:298-300` walks up from every
`subworkers/<name>/workspace` and would leak one file into all agent runs.

---

## PHASE SPLIT

| Phase | Scope | Demonstrable milestone |
|---|---|---|
| **P0** Skeleton + containment | Copy server, copy control plane, repo-local DB helper, uninstall guard, identity health, `git init`. Zero agents enabled. | `cargo build` green in `NEW/server`; `curl :5680/health` returns `{"status":"ok","repo":"subworker-opencode-rust"}`; `NEW/server/data/opencode/opencode.db` exists and `~/Library/Application Support/opencode/opencode.db` mtime unchanged. |
| **P1** One agent end-to-end | `test-delegator` only, disposable. Persona injection lever in Rust, prompt-prefix generator, SSE `prompt.final` waiter, timeout-abandon. | `./start.sh` → POST run → real SSE tool frames → run row `status=success` in `NEW/control/data/runs.jsonl`. No file written outside `NEW/subworkers/test-delegator/workspace/`. |
| **P2** Fleet + rewrite + docs | Path rewrite (2 anchors), all 22 identities + 5 imports, scheduler, `SUBWORKERS_SYSTEM.md`, Docker. | `GET /subworkers` lists 27 with exactly one `enabled:true`; `SUBWORKERS_SYSTEM.md` §11 prefix block matches disk 27/27. |

**Phase 1 is the gate. Phase 2 does not start until P1's gate is green.**

---

## WAVE 0 — SAFETY SCAFFOLDING (STRICTLY SERIAL, FIRST)

### W0.1 Blast-radius probe (read-only, no new files)

```bash
ls -1 ~/.omp/agent/agents/*.md | wc -l                                    # expect 52
ls -1 ~/.config/opencode/agents/*.md | wc -l                              # expect 46
stat -f '%z %m' ~/.config/opencode/opencode.json                          # expect 23096
ls -la ~/Library/'Application Support'/opencode/opencode.db               # record inode+mtime
ls -la ~/.omp/agent/agent.db | wc -l                                      # expect 8 corrupt gens
ls -A NEW | wc -l                                                         # expect 0 (empty repo)
```

**GATE:** all six print the expected value. Any deviation → STOP, re-derive.

### W0.2 Freeze sources

All work happens in `NEW`. Both source repos are **read-only inputs**; no task lists a source path
as a write target.

---

## WAVE 1 — COPY (4-WAY PARALLEL, disjoint targets)

### W1.1 `NEW/server/` ← opencode-server-rust

```bash
mkdir -p NEW/server
rsync -a --exclude='/target' --exclude='/.old' --exclude='/.codegraph' \
      --exclude='/.ulw' --exclude='/.git' --exclude='ralph-loop.local.md' \
      ~/Documents/opencode-server-rust/ NEW/server/
```

**GATE:** `find NEW/server -type f | wc -l` → 347 · `test -f NEW/server/crates/opencode-server/src/events.rs` → 0 (must exist; it is **untracked in source** and `lib.rs:4` declares it) · `test ! -e NEW/server/target` → 0 · Cargo.toml members == 18.

### W1.2 `NEW/control/` ← omp control plane + shell (Bun, verbatim copy-then-edit)

```bash
mkdir -p NEW/control
rsync -a --exclude='/node_modules' --exclude='/crates' --exclude='/packages' \
      --exclude='/data' --exclude='/bazel' --exclude='/nix' --exclude='/python' \
      --exclude='/target' --exclude='/.git' --exclude='Cargo.toml' --exclude='Cargo.lock' \
      --exclude='rust-toolchain.toml' --exclude='.env' --exclude='config/tunnel.token' \
      --exclude='config/proxies.txt*' --exclude='config/server.json' \
      --exclude='config/subworkers.json' \
      ~/Documents/omp-server/omp-server/ NEW/control/
cp ~/Documents/omp-server/{start.sh,install.sh,stop.sh,reset.sh} NEW/
rsync -a ~/Documents/omp-server/.omp NEW/.omp
rsync -a ~/Documents/omp-server/.opencode NEW/.opencode
```

`crates/pi-*` + root `Cargo.toml` are the **oh-my-pi** Rust workspace — a different project.
Copying them collides with `NEW/server/Cargo.toml`. `.env` and `tunnel.token` are excluded **at
copy time** (`rsync -a` ignores `.gitignore`, and `.env` contains the byte-identical **live**
`ELIA_AUTH_TOKEN`).

**GATE:** `test ! -e NEW/control/.env` → 0 · `test ! -e NEW/control/crates` → 0 · `ls NEW/control/*.ts | wc -l` → 15 · `grep -rl "70c0bb6a44c6c" NEW/ | wc -l` → **0**.

### W1.3 `NEW/subworkers/` ← omp's 22 agent homes, workspace-lite

```bash
mkdir -p NEW/subworkers
for d in ~/Documents/omp-server/subworkers/*/; do
  n=$(basename "$d"); [ "$n" = "context" ] && continue
  mkdir -p "NEW/subworkers/$n/workspace"
  rsync -a --exclude='/workspace' "$d" "NEW/subworkers/$n/"
  find "$d/workspace" -maxdepth 2 -type f \( -name '*.md' -o -name '*.txt' \) -size -256k \
    -exec rsync -a --relative "{}" "NEW/subworkers/$n/" \; 2>/dev/null
done
rsync -a ~/Documents/omp-server/subworkers/context NEW/subworkers/context
```

omp's `subworkers` total is 1.1 GB and `app-clone-factory/workspace` alone is 1.7 GB.
**Run-output blobs do not come over.** Historical run logs must not be rewritten anyway.

**GATE:** `ls -d NEW/subworkers/*/workspace | wc -l` → 22 · `ls NEW/subworkers/*/PROMPT.md | wc -l` → 22 · `du -sh NEW/subworkers` → **< 60 MB** · `test -e NEW/subworkers/elia/PROMPT.md` → 0.

### W1.4 `NEW/config-home/agents/` ← `~/.omp/agent/agents` (52 personas)

```bash
mkdir -p NEW/config-home/agents
rsync -a ~/.omp/agent/agents/ NEW/config-home/agents/
```

**GATE:** `ls NEW/config-home/agents/*.md | wc -l` → **52** · `grep -c "OMP HOME" NEW/subworkers/refund-hunter/PROMPT.md` → 1.

### Wave-1 joint gate (blocking)

`grep -rn "ChrisFeldmeier/OpenCodeRust" NEW/ --include='*.toml' --include='*.json' | wc -l` → 0
· `git -C NEW remote -v` → **empty/absent** (fresh `git init`, **no remote**).

---

## WAVE 2 — CONTAINMENT PRIMITIVES (3-WAY PARALLEL)

### W2.1 Rust: single path-root helper — TDD FIRST

**New:** `NEW/server/crates/opencode-paths/{Cargo.toml,src/lib.rs,tests/root.rs}`
**Edit:** `NEW/server/Cargo.toml` (add member)

**Six sites repointed, each KEEPS its own fallback:**

| Site | Line | Own fallback |
|---|---|---|
| `crates/opencode-storage/src/database.rs` | 113-119 | `PathBuf::from(".")` ⚠️ cwd-relative |
| `crates/opencode-cli/src/main.rs` | 2242-2247 | `PathBuf::from(".")` |
| `crates/opencode-cli/src/main.rs` | 2679-2684 | uninstall targets (→ W2.2) |
| `crates/opencode-server/src/server.rs` | 375-388 | `temp_dir` |
| `crates/opencode-mcp/src/auth.rs` | 52-57 | `PathBuf::from(".")` |
| `crates/opencode-config/src/wellknown.rs` | 46-51 | `~/.local/share` |

Env-var only, no code: `instruction.rs:65-73` — `OPENCODE_CONFIG_DIR` takes precedence.

**TDD (write first, watch fail):** 5 cases — `OPENCODE_PATHS_ROOT` honoured · unset → caller's
fallback preserved verbatim · empty/whitespace → fallback · relative root resolved at test time ·
and the **two-suffix invariant**:

```rust
assert_eq!(paths::provider_auth_dir("/R"), "/R/opencode/data");
assert_eq!(paths::db_file("/R"),        "/R/opencode/opencode.db");
```

**GATE:** `cd NEW/server && cargo test -p opencode-paths` → `5 passed`; `cargo build` → exit 0.

### W2.2 Rust: uninstall blast-radius guard — TDD FIRST  ⚠️ HIGHEST RISK

**Files:** `NEW/server/crates/opencode-cli/src/main.rs:2675-2710`; test `tests/uninstall_guard.rs`

If `OPENCODE_PATHS_ROOT` is set, `uninstall` targets **only** that root and **refuses**
data/cache/config/state unless `--force --yes-i-mean-it`. Print the resolved absolute target of
every path before `remove_dir_all`. Keep `--dry-run`.

**Why:** `main.rs:2679-2684` currently targets `~/.config/opencode`, which holds all **46**
personality `.md` + the 23,096-byte `opencode.json`. And once the DB is repo-local, uninstall run
from `NEW` still deletes the OLD repo's live state.

**TDD (must fail before fix):** (a) with `OPENCODE_PATHS_ROOT` set, `uninstall --dry-run` lists
**no** `~/.config/opencode`; (b) without it, behaviour unchanged; (c) `--force` alone still refuses.

**GATE:** `cargo test -p opencode-cli --test uninstall_guard` → `3 passed`.
**No Wave-3 task may run before this is green.**

### W2.3 Control plane: identity health + fresh token

**Files:** `NEW/control/routes/serverRoutes.ts:7-18`, `NEW/control/server.ts:52-54`, `NEW/start.sh:24,51-60`

- `/health` returns `{status, repo, db_path, agents_count, uptime_s}`; `repo` is a const
  `"subworker-opencode-rust"`.
- `/server/health` stops returning the hardcoded `"healthy"`; it reads the child Rust `/health`.
- `start.sh` asserts `jq -r .repo == "$REPO_ID"` before declaring healthy and **exits 1** on
  mismatch (today it exits 0 — the silent-green hazard).
- `.env` created by `install.sh` only, mode 600, **freshly minted** `ELIA_AUTH_TOKEN`.

**TDD:** `NEW/control/tests/health_identity.test.ts` — `/health` has `repo`; `server_healthy()`
fails when `repo` ≠ `REPO_ID`.

**GATE:** `cd NEW/control && bun test tests/health_identity.test.ts` → `2 pass` ·
`grep -o 'ELIA_AUTH_TOKEN=..\{12\}' NEW/.env` ≠ `70c0bb6a44c6c`.

---

## WAVE 3 — FIRST-GATE SMOKE: ONE DISPOSABLE AGENT (FULLY SERIAL)

**Yes, gate before the bulk rewrite.** The rewrite is ~150 edits across 40 files with two anchors
and 8 do-not-touch files in the same blast zone. P1 is where you discover persona injection needs
a **Rust code change** (`routes.rs:1704` passes `system: None`; `AgentParams` at
`prompt.rs:129-133` has no persona field) — it does. Rewrite first and you edit 22 prompts twice
with no way to attribute a break. The smoke costs one agent; the rewrite costs a day.

### W3.0 Fix the dangling ref BEFORE the run
`NEW/subworkers/test-delegator/PROMPT.md:7` — `subworkers/SUBWORKERS_SYSTEM.md` → `docs/SUBWORKERS_SYSTEM.md`.
**GATE:** `grep -c "subworkers/SUBWORKERS_SYSTEM" NEW/subworkers/test-delegator/PROMPT.md` → **0**.

### W3.1 `git init` + first commit
```bash
cd NEW && git init -q && git add -A && \
  git commit -q -m "chore: scaffold subworker-opencode-rust (server copy + control plane + 22 agent homes)"
```
**GATE:** `git -C NEW remote -v | wc -l` → **0** · `git -C NEW status --porcelain | wc -l` → 0.

### W3.2 Rust: persona/system-prompt injection — TDD FIRST

**Files:** `NEW/server/crates/opencode-server/src/routes.rs:1686-1704`,
`NEW/server/crates/opencode-agent/src/prompt.rs:129-133`, test `tests/persona_injection.rs`

Add `system: Option<String>` to the prompt body; `run_prompt_turn` prepends it when present. Also
honour an optional `persona_file` resolved **inside `directory` only** (reject `..`, reject
absolute, require the resolved path to stay under `directory`).

**TDD:** (a) no `system` → unchanged (regression); (b) `system:"P"` → first system message == `P`;
(c) `persona_file:"../../etc/passwd"` → **400**, no read; (d) inside `directory` → injected;
(e) `wire_contract.rs` stays green.

**GATE:** `cargo test -p opencode-server --test persona_injection --test wire_contract` → 0 failed.
**Most important behavioural gate in the plan.**

### W3.3 Control plane: prompt-prefix generator — TDD FIRST

**New:** `NEW/control/promptPrefix.ts`; test `NEW/control/tests/prompt_prefix.test.ts`

Pure function `prefixFor(name, repoRoot)` → the verbatim `> OMP HOME:` line-2 spec with ONE
substitution (the path), **plus** the `## Workspace Constraint` `workspace/` path at line ~6.
**BOTH anchors.** No templating — `HOWTOADDAGENTS.md:29-30` states injection is VERBATIM.

**TDD:** 22/22 round-trip byte-identical after path swap · 0 `EliaAI/subworkers` residues ·
`refund-hunter`'s 64 KB **not truncated** (do NOT reproduce the 12k cap at `worker.ts:223-225`).

**GATE:** `bun test tests/prompt_prefix.test.ts` → `1 pass`, assertion count ≥ 22.

### W3.4 Control plane: HTTP engine + `prompt.final` waiter — TDD FIRST

**New:** `NEW/control/httpEngine.ts`; test `NEW/control/tests/http_engine.test.ts`

`POST /session {directory, agent}` → `POST /session/{id}/prompt` → consume `GET /event` SSE →
resolve on `{"type":"session.updated","source":"prompt.final"}`.
**`session.idle` is NEVER awaited** (`routes.rs:1763` uses `notify_plugin_event`, plugins only).
`timeout_minutes` = abandon the HTTP request; the turn is `tokio::spawn`ed (`routes.rs:1803`) so it
always finishes and always emits `prompt.final`. `POST /prompt` returns 200 before Busy is set
(`:1817` vs `:1640`) — poll only for recovery, never as the primary signal.
`OMP_AGENT_DIR=NEW/config-home` exported via `.env` (`set -a; . .env; set +a`).

**TDD:** (a) resolves on `prompt.final`; (b) **hangs** on an idle-only stream → assert timeout
fires (proves we don't await idle); (c) timeout records `timeout`, not `success`; (d) never POSTs
to `/abort` expecting an effect (`routes.rs:1825-1846` is a no-op).

**GATE:** `bun test tests/http_engine.test.ts` → `4 pass`.

### W3.5 THE FIRST-GATE RUN

```bash
cd NEW && ./start.sh          # binds 127.0.0.1:5680
curl -s http://127.0.0.1:5680/health | jq -e \
  '.repo=="subworker-opencode-rust" and (.db_path|startswith("'"$PWD"'/server/data"))'
```

**GATE — all five simultaneously:**

1. `jq -e '.repo=="subworker-opencode-rust"'` → exit 0
2. `test -f NEW/server/data/opencode/opencode.db` → 0 **AND** `stat -f '%m' ~/Library/'Application Support'/opencode/opencode.db` **unchanged from W0.1**
3. `ls NEW/subworkers/test-delegator/workspace/ | wc -l` ≥ 1 after the run
4. `find NEW/subworkers/test-delegator/workspace -newermt '-10 minutes' | wc -l` ≥ 1 **AND** `find ~/Documents/omp-server ~/EliaAI -newermt '-10 minutes' -not -path '*/workspace/*' | wc -l` → **0**
5. `tail -1 NEW/control/data/runs.jsonl | jq -e '.name=="test-delegator" and .status=="success"'` → exit 0

**If any of 1-5 fails, STOP. Do not start Phase 2.**

---

## WAVE 4 — PHASE 2: REWRITE + FLEET + DOCS

### W4.1 Path rewrite pass A — omp anchor (∥ W4.2)
Scope: `NEW/subworkers/*/PROMPT.md` (22 files, 51 hits), `NEW/control/config/subworkers.json`
(22 `workspace` values), `NEW/.omp` (41 lines/6 files), `NEW/.opencode` (36 lines/5 files), `NEW/.gitignore`.
`/Users/vakandi/Documents/omp-server` → `NEW`

**MUST NOT rewrite:** `/Users/vakandi/EliaAI/context` (8 files), `/Users/vakandi/EliaAI/memory`,
`~/.config/opencode/agents`, `~/.config/omp/agents`, MirorPay / VcamSaas / mcps_server / OpenRF
paths, `/data/subworkers` (3 container paths).
**Never touch:** `NEW/subworkers/*/workspace/**` (131 files carry historical absolute paths — that
is a record, not config).

**GATE:** `grep -rl "Documents/omp-server" NEW --exclude-dir=workspace --exclude-dir=.git | wc -l` → **0**
· `grep -rl "EliaAI/context" NEW/subworkers/*/PROMPT.md | wc -l` → **8** (preserved)
· `grep -rl "Documents/omp-server" NEW/subworkers/*/workspace | wc -l` → **131** (unchanged, proves exclusion).

### W4.2 Path rewrite pass B — EliaAI anchor, 5 fresh imports
Import `bene2luxe-promoter`, `bene2luxe-suppliers`, `cobou-promoter`, `tempack-dev`,
`tiktok-content` from `~/EliaAI/subworkers` with
`--exclude='/server' --exclude='/workspace' --exclude='/.codegraph' --exclude='/.omo' --exclude='/logs' --exclude='/scripts'`.
**`/server` is mandatory:** `~/EliaAI/subworkers/server/opencode-data/opencode.db` is **1.4 GB**
live SQLite with credential tables.
Then rewrite `/Users/vakandi/EliaAI/subworkers` → `NEW/subworkers` and inject **both** prefix anchors.

**GATE:** `find NEW -name opencode.db -size +100M | wc -l` → **0** ·
`grep -rl "EliaAI/subworkers" NEW/subworkers/{bene2luxe-promoter,cobou-promoter,tempack-dev,tiktok-content} | wc -l` → 0.

### W4.3 Registry: 27 entries, 1 enabled, no dead config
- Every entry: explicit `variant` (fix the **4** nulls: `mirrorpay-telegram`, `reddit-saas-scraper`,
  `vcam-community-organic`, `vcam-saas`), `workspace` = absolute `NEW/subworkers/<name>/workspace`,
  **`prompt_file` deleted** (dead — `engine.ts:91` reads the file),
  `enabled:false` **except** `cve-hunter:true`.
- All-false is deliberate: containment satisfied at 0% behaviour. Ship **one** disposable enabled agent.

**GATE:** `jq -e '[.subworkers[]|select(.enabled==true)]|length==1'` → 0 ·
`jq -e '[.subworkers[]|select(.variant==null)]|length==0'` → 0 · `jq -e '.subworkers|length==27'` → 0 ·
`grep -c prompt_file` → 0.

### W4.4 Rename fixes — `omp-server/` → `server/` (4 sites, ATOMIC)
`NEW/start.sh:21` · `NEW/install.sh:27-28,40-41` · `NEW/control/engine.ts:364` (spawn literal) ·
`NEW/control/worker.ts:275` (`resolveExtensionsDir`) · `NEW/control/docker-compose.yml:4-5`
(`build.dockerfile`, relative to `context: ..`).
**Do NOT rename `subworkers/`** — `auth.ts:110-136` `effectiveWorkspace()` builds 4 candidates from
the literal `"subworkers"` and falls through **silently** on a miss. The folder name is an API contract.

**GATE:** `grep -rn '"omp-server/' NEW --include='*.ts' --include='*.sh' --include='*.yml' | wc -l` → **0** ·
`cd NEW/control && bun test` → 0 fail.

### W4.5 Docker
Move `NEW/control/docker-compose.yml` → `NEW/docker-compose.yml`. Add `XDG_CONFIG_HOME` +
`OPENCODE_PATHS_ROOT=/srv/server/data`. `env_file` becomes **required** (drop `required:false` —
a missing `.env` silently yields ZERO isolation). Keep `${SUBWORKERS_DIR:-…}` for the read-only
reference tier (already parameterised). Keep `ELIA_AUTH_TOKEN: ${ELIA_AUTH_TOKEN:?…}` (hard-fails).

**GATE:** `docker compose -f NEW/docker-compose.yml config` → exit 0, output contains
`OPENCODE_PATHS_ROOT` and **not** `required: false`.

### W4.6 `SUBWORKERS_SYSTEM.md` (11 sections, not 8)
`NEW/docs/SUBWORKERS_SYSTEM.md`. Fix known staleness: "25 entries"→27 · "all `enabled:false`"→"1
enabled" · `workspace: /data`→real absolute paths.
§11 = the prompt-prefix deliverable: table of all 27 agents with their exact injected line-2 path.
Import **only** `AGENTS.md:85` (elia env inventory) and `:94` (elia verification script); **skip**
Key Directories / Dev Commands / Code Conventions (100% upstream-Bun, actively misleading in a Rust repo).

**GATE:** `grep -c '^## ' NEW/docs/SUBWORKERS_SYSTEM.md` → **11** ·
`grep -c "25 entries\|all enabled" NEW/docs/SUBWORKERS_SYSTEM.md` → 0 ·
`grep -c 'workspace: /data' NEW/docs/SUBWORKERS_SYSTEM.md` → 0.

### W4.7 Root `AGENTS.md` — DELIBERATELY OMITTED
`instruction.rs:298-300` walks up from every workspace into all 22 runs. `opencode-server-rust`
has none, so nothing leaks today. **Do not create one.** Document the reason in this file.
**GATE:** `test ! -e NEW/AGENTS.md` → 0.

---

## WAVE 5 — FINAL ACCEPTANCE (serial)

```bash
cd NEW/server && cargo test --workspace
cd NEW/control && bun test
```

**GATE — all must hold:**
1. `cargo test --workspace` → **0 failed** (includes `wire_contract`, `sse_livestream`, `persona_injection`, `uninstall_guard`, `paths`)
2. `bun test` → 0 fail
3. `grep -rn "70c0bb6a44c6c\|tunnel.token=" NEW --include='*.env' --include='*.ts' --include='*.sh' | wc -l` → 0
4. `git -C NEW status --porcelain | wc -l` → 0 · `git -C NEW remote -v | wc -l` → 0
5. Re-run **W0.1 verbatim** → all six values identical. *This is the "nothing live was harmed" proof.*
6. `./stop.sh` then `./start.sh` → prints `already running` **only** when `.repo` matches

---

## SAFETY ORDERING (irreversible steps)

| # | Step | Gate before | Gate after | Blast radius if it fails |
|---|---|---|---|---|
| 1 | `uninstall` guard (W2.2) | 3 failing TDD cases | `3 passed` | **46 personalities + 23 KB opencode.json.** Highest risk. Wave 2, before any other Rust edit. |
| 2 | DB relocation (W2.1) | env unset → old path bit-identical | inode/mtime of the live DB unchanged | Stray DB, or old DB stops being written. Both detectable. |
| 3 | Port `:5680` (W2.3) | `lsof -iTCP:5680` empty | `/health` `.repo` matches | Silent-green against the old server. Asserted, not assumed. |
| 4 | Token mint (W2.3) | `.env` absent | new token ≠ `70c0bb6a44c6c…`, mode 600 | New repo could authenticate to live omp. |
| 5 | Path rewrite (W4.1/4.2) | script dry-runnable, prints every file | 0 omp hits **and** 8 context hits **and** 131 workspace hits | Agents write into omp/EliaAI. Mitigated by W3.5 gate 4. |
| 6 | rsync copy (W1.x) | `NEW` empty, `git init` no remote | 347 files, `events.rs` present, 0 secrets, <60 MB | Truncated tree cannot compile. Gate catches it. |

Only #1 must precede everything. #5 is last because it is the widest and cheapest-to-redo edit.

---

## PARALLELISM MAP

```
W0.1 probe                       SERIAL (read-only baseline)
W1.1 ∥ W1.2 ∥ W1.3 ∥ W1.4        4-way parallel, disjoint targets
W2.1 ∥ W2.2 ∥ W2.3               3-way parallel, disjoint crates/files
W3.0→W3.1→W3.2→W3.3→W3.4→W3.5  FULLY SERIAL (each gate is the next precondition)
W4.1 ∥ W4.2 ∥ W4.3               partial parallel, disjoint file sets
W4.4 → W4.5 → W4.6 → W4.7        serial (rename atomic across 4 sites)
W5 acceptance                    serial
```

**W3 serial:** every step touches the prompt-assembly path
(`routes.rs:1686-1704` → `prompt.rs:129-133` → `promptPrefix.ts` → `httpEngine.ts`).
Parallelising produces unattributable failures.

---

## TARGET TREE

**After P0:**
```
subworker-opencode-rust/
├── .git/  .gitignore  .omp/  .opencode/  .env(gitignored,600)
├── docs/PLAN.md                        ← this file
├── start.sh  stop.sh  reset.sh  install.sh
├── config-home/agents/*.md             (52)
├── subworkers/<22>/{PROMPT.md,workspace/} + subworkers/context/
├── control/   (15 .ts + routes/ + config/ + docs/ + Dockerfile + package.json)
│   └── data/                            (gitignored)
└── server/
    ├── Cargo.toml  Cargo.lock  README.md  USER_GUIDE.md  opencode.jsonc
    ├── crates/… (18 members)
    ├── crates/opencode-paths/          ← NEW
    └── data/opencode/                  ← runtime, gitignored
```

**P1 adds:** `server/crates/opencode-server/tests/persona_injection.rs`,
`server/crates/opencode-cli/tests/uninstall_guard.rs`, `control/promptPrefix.ts`,
`control/httpEngine.ts`, `control/tests/{prompt_prefix,http_engine}.test.ts`

**P2 adds:** `docker-compose.yml` (root), `docs/SUBWORKERS_SYSTEM.md`, `subworkers/<5 new>/`,
`control/config/subworkers.json` → 27 entries

**Explicitly NOT created:** `AGENTS.md` anywhere · `server/.old/` · `server/target/` ·
any `.db` outside `server/data/` · any `.omp/agent/agent.db` copy.

---

## COMMIT STRATEGY

One commit per wave task, **only after** that task's gate is green. Conventional commits.
Never commit `.env`, `*token*`, `server/data/`, `control/data/`, `server/target/`, `*.db`.
**No remote, ever.** If a gate fails: `git reset --hard HEAD~1`. Never amend a green commit.

| # | Message | Gate |
|---|---|---|
| 1 | `chore: scaffold subworker-opencode-rust from omp-server + opencode-server-rust` | W1 joint |
| 2 | `feat(paths): add opencode-paths crate with repo-relative root resolution` | paths 5 passed |
| 3 | `fix(cli): make uninstall repo-aware and refuse shared config dirs` | uninstall_guard 3 passed |
| 4 | `feat(control): identity-asserting health + fresh token minting` | health_identity 2 pass |
| 5 | `feat(server): persona/system-prompt injection on the prompt endpoint` | persona_injection + wire_contract |
| 6 | `feat(control): HTTP engine driving prompt.final over SSE` | http_engine 4 pass |
| 7 | `feat(control): generate per-agent OMP HOME + workspace prompt prefix` | prompt_prefix 1 pass |
| 8 | `test: first-gate smoke run of test-delegator` | W3.5 gates 1-5 |
| 9 | `refactor(subworkers): rewrite omp-server absolute paths to subworker-opencode-rust` | W4.1 |
| 10 | `feat(subworkers): import 5 EliaAI-only agents with rewritten prefixes` | W4.2 |
| 11 | `chore(config): registry to 27 entries, explicit variants, 1 enabled` | jq gates |
| 12 | `refactor: rename omp-server/ segment to server/ across 4 call sites` | 0 residual + bun test |
| 13 | `chore(docker): root compose, required env_file, OPENCODE_PATHS_ROOT` | `docker compose config` |
| 14 | `docs: SUBWORKERS_SYSTEM.md with 11 sections and 27-agent prefix table` | 11 headings, 0 stale |

---

## LIVESTREAM CONTRACT — EliaTopBar (macOS menu-bar app)

> Added 2026-10-07. The macOS app is **EliaTopBar**, at `~/Documents/EliaTopBar`
> (Swift, `Sources/`). It does NOT talk to the Rust server directly. It opens a
> WebSocket to the **Bun control plane** and reads `run_log` frames. So the contract
> that matters is the one `control/engine.ts` emits, not the Rust SSE.

### Frame path

```
Rust  GET /event SSE            control/httpEngine.ts      control/engine.ts        EliaTopBar
────────────────────────        ────────────────────       ──────────────────        ──────────
message.part.delta / .updated → Frame{field,text,tool,   emitRunLog(name,field,   run_log WS
                                  input,output}             JSON delta)              → appendDelta
```

`engine.ts` emits the tool delta as JSON so the app can parse it:

```json
{"tool": "bash", "input": {"command": "pwd"}, "output": "/path"}
```

### What each panel parses (verified against the shipped Swift)

- **RunPopup** — `RunPopupController.swift` + `Livestream/LivestreamStore.swift:48`.
  Consumes `run_log` `{name, text, field}`; `field` ∈ `text|reasoning|tool`.
  `appendDelta` keeps a single rolling entry per field and drops a delta that repeats
  the current value (`LivestreamStore.swift:56`) — which is why our text/reasoning
  de-duplication in `httpEngine.ts` matters: duplicate frames are dropped by the app,
  but they corrupt the todo and subagent parses.
- **LogViewer subagent panel** — `LogPopoverView.swift:843` `extractSubagents`.
  A subagent bubble is created when a tool frame carries a session id. Two triggers:
  - `call_omo_agent` / `task` → the output must contain `Session ID: ses_...`
    (regex at `LivestreamStore.swift:317`).
  - `team_create` → the output must contain `runtimeState.members[].sessionId`.
  It then re-parses `input` for a human label (`LogPopoverView.swift:1405-1431`):
  `task` reads `prompt` then `description`; `websearch` reads `query`;
  `bash`/`edit`/`grep`/`glob`/`write` read their own keys. **A tool frame whose
  `tool` is missing renders as `▸ tool` and never spawns a bubble** — hence the
  `callID` correlation in `httpEngine.ts`.
- **Todo panel** — `LivestreamStore.swift:86` → `LivestreamParsing.extractTodos`
  (`LivestreamParsing.swift:75-96`). Recursively looks for a `todos` array whose
  items carry `content`, `status`, `priority`. Works whether `todos` sits at the top
  level of the delta or nested under `input`.

### Regression test

`control/tests/topbar_contract.test.ts` replays the Swift decoders in TypeScript
against the frames `httpEngine.ts` emits. It is the only test that proves the macOS
app can render this server's output — a green SSE stream alone proves nothing about
the UI. 12 assertions: todo extraction, task label fields, `team_create` member
session ids, the `Session ID:` regex, reasoning de-duplication, and no nameless tool
frames.

**Run:** `cd NEW/control && bun test tests/topbar_contract.test.ts`

---

## OPEN QUESTIONS FOR WAEL

1. **`~/.omp/agent/` has 8 `agent.db.corrupt-*` generations (~5.5 MB) clustered around Sep 20-21.**
   That suggests a prior corruption event. Confirm before copying that layout into a new repo.
2. **`:5680` was free during planning but omp was NOT listening on `:5677`.** Confirm live omp is
   actually running before W3.5, or the identity assertion goes untested against a real neighbour.
3. **`mcp_servers` is non-empty on exactly one agent (`boxing-audience-scout`) and cannot be
   honoured per-session** (MCP is process-global, `routes.rs:2535-2542`). Accept the limitation or
   drop the field?
4. **Phase 2 excludes scheduler parity** (`every`/`hours` active-window semantics have no Rust
   equivalent to test against). Confirm deferral is acceptable.
