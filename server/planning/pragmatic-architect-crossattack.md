# CROSS-ATTACK — pragmatic-architect · Round 3

**Directive:** (A) determine whether the central tension resolves in favour of feasibility-skeptic's
configuration thesis or perf-architect's language thesis; (B) attack the weakest link in the
rewrite+adopt-`/tmp/OpenCodeRust` proposal — if both language-independent defects are real, is the ref
worth ANY adoption?

**One-line answers:** **(A) Neither — the tension is a false dichotomy over a category error: three
agents measured three different memory pools and compared them as one budget.** **(B) Yes for five
subsystems, no as a drop-in — and the ref's persistence layer is a data-loss vector on the actual
fleet data.**

I converge with feasibility-skeptic on the single most important finding (§D). I do not converge on his
remaining arithmetic, and one number he is still relying on is wrong (§B.4).

---

## A) THE CENTRAL TENSION RESOLVES

### A.1 The category error, measured

Three pools, one assumed budget:

| | perf-architect | feasibility-skeptic | **target** |
|---|---|---|---|
| Artifact | `~/.local/share/opencode/opencode.db` | `vouch-opencode` container | **`elia-subworker-srv`** |
| DB size | **18,997,758,400 B (17.69 GiB)** | 4,997,120 B (4.8 MiB) | **1,405,136,896 B (1.31 GiB)** |
| Memory ceiling | none (host process) | **1,258,291,200 B = 1200 MiB** | 8 GiB |
| Swap | n/a | **1200 MiB** | **zero** (`memswap_limit: 8g` = `mem_limit: 8g`) |
| Sessions | 3,126 | — | **1,019** |
| Newest session | **2026-10-05 00:38** | — | **2026-09-19 12:27** (16 days stale) |
| Fleet? | **no** | **no** | **yes** |

```
$ docker inspect elia-subworker-srv → error: no such object: elia-subworker-srv
$ docker ps → vouch-opencode, openrf-worker, openrf-fastapi, directus-local,
              openrf-cloudflared, openrf-dashboard          # no subworker container
```

### A.2 perf's #1 hotspot is absent from the fleet's pool

Same `dbstat` method on both DBs:

| table | **fleet DB (1.31 GiB)** | **host TUI DB (17.69 GiB)** |
|---|---|---|
| `event` | **0 MiB ⚠️ dbstat unreliable here — see A.2a** | **15,309 MiB** |
| event indexes (3) | 53 MiB | 229 MiB |
| `part` | **406 MiB** | 1,598 MiB |
| `message` | 11 MiB | 889 MiB |
| **`part`+`message`** | **417 MiB (31.8%)** | **2,487 MiB (14.1%)** |

**The 14.94 GiB "durable, never-pruned, never-read" event log — perf's #1 RAM hotspot and the team's
biggest claimed win — does not exist in the database the fleet actually opens.** Not small: absent from
`dbstat` entirely, while `session` reads cleanly from the same file.

### A.2a ⚠️ RETRACTION (Round 5) — `dbstat` is unreliable on this corrupt DB, so the claim above is FALSE

I ran a second query and **falsified my own §A.2.** `dbstat` **aborts mid-scan** on this corrupt
database. An alphabetical full walk lists `event` at 0 MiB *and* its three indexes at 53 MiB — **an index
cannot exist without its table**, so `event` is present and the scan merely stopped reporting its pages.

```
SELECT SUM(pgsize) FROM dbstat   ->  498 MiB
page_count x page_size           ->  1340 MiB      # dbstat accounts for 37% of the file
```

**Therefore:** (a) per-table `dbstat` figures on this database are **unreliable** and my "absent"
inference was invalid — the same conclusion I had already flagged as my #1 unverified claim, now
falsified by my own next query rather than resolved; (b) **perf's ~15 GiB event-log claim is UNMEASURED
here, not refuted** — I withdraw the part of §A.2 where I said it "does not exist"; (c) the pool-level point
survives only on the `page_count` figure (1.31 GiB vs 17.69 GiB). **Settle this with `.recover`, which
reads b-tree pages directly, never with `dbstat`.** ts-anatomy's addendum reaches the same conclusion by
the opposite route (`event` fails `COUNT(*)` = corrupt-but-present), so **two of us now agree the table
exists and neither of us has sized it.**

**⇒ The fleet's entire message history is 11 MiB across 1,019 sessions (~11 KiB/session).** perf's
83.5 MiB worst-case session belongs to a pool whose total history is 11 MiB. His §1b transient-spike
analysis and my §1.2 unbounded-RAM finding are both aimed at a problem **~50× smaller in this pool.**

### A.3 skeptic's baseline is a different product with 10× the memory and real swap

He measures `vouch-opencode` — **1200 MiB limit, 1200 MiB swap**, DB **4.8 MiB**, 3 MCP servers — and
extrapolates to the target: **8192 MiB, zero swap**, DB **1.31 GiB**, plus Python FastAPI, APScheduler,
cloudflared, MCP children and Chromium sharing one cgroup.

His **per-unit constants remain the best measurements on the team** (465.3 MB idle Bun baseline,
49.7 MB/MCP child, both from `/proc/<pid>/status` on a live container). I use them below. His
**aggregates** are built against the wrong deployment.

### A.4 Verdict on the tension

| Question | Answer | Basis |
|---|---|---|
| Is the fleet's 8 GB currently blown? | **Unknown — unmeasured** | Container absent from `docker ps` |
| Is the 18.99 GB event log the fleet's problem? | **No** | Absent from fleet DB (§A.2) |
| Is per-directory fan-out unbounded + never evicted? | **Yes, structurally** | `instance-state.ts:31` + 22 sites; `/instance/dispose` → **404 verified** |
| Does `part`=406 MiB imply a history problem? | **At ~0.4 MiB/session, no** | 406 MiB ÷ 1,019 |
| Is Rust right for the hot path? | **Unproven either way** | No measurement exists on any pool |

**Both language-independent defects are real** ⇒ they are the correct first work regardless of language
(**skeptic wins on priority**). **But every magnitude that made the rewrite look urgent came from the
wrong pool**, where the numbers are ~50× smaller. **Neither thesis is confirmed. The discriminating
measurement has never been run.**

---

## B) WEAKEST LINK IN "REWRITE + ADOPT THE RUST REF"

Adoption is a seven-part claim. Five survive my own audit; two do not.

### B.1 Survives

| Claim | Evidence |
|---|---|
| Builds | `cargo check --workspace --all-targets` → **EXIT=0**, 0 errors, 40s |
| Real streaming loop | `prompt.rs:937 loop_inner` — cancellation, compaction, plugin hooks, incremental deltas |
| Real provider layer | 17,336 LOC, 33 files, 20+ adapters |
| Real tools | 18 registered (`registry.rs:223-250`) |
| Real HTTP surface | 99 routes; `/global/health` → **200**, `POST /session` → works, `x-opencode-directory` middleware present |
| Storage schema | SQLx/SQLite, 6 tables, full CRUD |
| Not a skeleton | 0 `todo!()`, 0 `unimplemented!()` |

### B.2 W1 — persistence is a data-loss vector on the actual fleet data

`server.rs:197 sync_sessions_to_storage()`, called from `routes.rs:236`: for **every** session,
`message_repo.delete_for_session(id)` then re-`create()` **every message**, having first deep-cloned
the entire session list (`manager.list().into_iter().cloned().collect()`).

Against a fleet DB whose b-trees are corrupt (§C1), adopting this as-is means **DELETE all `part` rows and
re-insert from an in-memory `HashMap`.** If in-memory state is incomplete — and this DB demonstrates
corruption is real — **that is unrecoverable total loss.** It also deep-clones every session every
sync: a **2× RAM spike** over the entire fleet.

**This alone disqualifies drop-in adoption.** It converts "adopt the ref" into "adopt the crates,
replace persistence" — which is most of a rewrite of the layer that matters.

### B.3 W2 — ~~the plugin handshake has never completed~~ **WITHDRAWN (Round 4).**

**plugin-host proved the pristine host loads REAL omo: 12 hooks + 26 tools.** My original claim rested on
my own 30s timeouts, which were a **cold-start artifact**, and on a *deterministic* hang that was in fact an
**intermittent lost-wakeup race** (`plugin-host.ts:96-103`). **The surviving part of W2 is smaller and is
about agents, not hooks:** `/agent` returns 3 hardcoded agents where TS returns 46 personas, and
`opencode_client.py` passes `agent:` on every message — cause still unisolated.

~~plugin-host root-caused it: `readExact()` awaits a `"readable"` event that **Node emits and bun does
not**.~~ **CORRECTED (Round 4):** the event fires on **both** runtimes; the fault is a **lost-wakeup race**
(`plugin-host.ts:96-103` — `read()` first, `once("readable")` attached only after `null`). So **not every
plugin load times out**, and `detect_runtime()` ordering is irrelevant.

**The consequence that does survive:** `/agent` returns **3 hardcoded agents** where TS returns **46
personas** (`~/.config/opencode/agents/*.md`, incl. `gilfoyle.md`), and `opencode_client.py` passes
`agent:` on **every** message. Root cause still unisolated — I retracted two wrong guesses of my own
(wrong crate; then wrong directory claim) and the honest status is *"the loader is ported at
`config/loader.rs:615`, the merge chain reads correct, and it still yields 3."* Until diagnosed, **every
subworker silently falls back to a default agent**: no persona, no per-agent tools, no per-agent model.
For 25 agents that is not a bug, it is a different product.

### B.4 The 54-server multiplier is still wrong — and skeptic did not address it

Round 2 I showed opencode never reads `~/.config/mcp/mcp_servers.json`: `grep -rn mcp_servers` → **zero
hits repo-wide**; `mcp/index.ts:496,595,671` read only `cfg.mcp`; `~/.config/opencode/opencode.json` has
**3** entries (`parallel-browser-mcp`, `undetected-browser`, `playwright`); `opencode_client.py:36-45`
lists 6 of which **5 are opencode built-ins**.

**His cross-attack still prices the MCP fleet at 54** — `feasibility-skeptic-crossattack.md:135-136`
("Against **54 configured servers** (`mcp_servers.json`) that is **2,684 MB**"), repeated at `:197` and
`:301`. He rebuts plugin-host and perf-architect but never addresses this correction to his own §1.

**Corrected:** `25 dirs × 3 external children × 49.7 MB ≈ 3.7 GB` — an **~18× reduction**, and it lands
where perf's per-directory Chromium estimate lands (2.7-4.4 GB). **His conclusion ("the MCP fleet
dominates") survives on ~3 of 54 servers; his number does not.**

Caveat I owe him: I did **not** audit all 25 `workspace/opencode.json` files for their own `mcp` blocks.
If some declare their own, the count rises — bounded by what those files declare, not by 54.

### B.5 W3 — hook surface is partial, and it fails silently

`subprocess/mod.rs:24-42` is an allowlist of **13** hook names; `host/plugin-host.ts:422` discards
`event` outright. plugin-host's defensible figure: **4 of omo's 14 hooks missing, 2 of which gate 16 of
omo's 56 named features** — including `experimental.compaction.autocontinue` ⇒ `preemptive-compaction`
dead. **I retracted my own inflated magnitude here in Round 2** (I had measured omo's event emissions,
not subscriptions). Direction holds; the failure mode is the problem: **silent no-op, not an error.**

### B.6 Verdict on adoption

**Adopt the crates: NO. Adopt them selectively, after three fixes: YES.**

| Keep (expensive to re-derive, not the bottleneck) | Replace (decides the goal) |
|---|---|
| provider streaming (17k LOC) | **persistence layer** (B2 — data-loss vector) |
| tool implementations (8.9k LOC) | **session-state residency** (§1.2 — unbounded RAM, 2× resync spike) |
| SQL schema + repositories | **startup ordering** (§1.3a — 70-90 s pre-bind) |
| HTTP route structure | **plugin handshake + hook allowlist** (B.3, B.5) |
| | **agent resolution** (B.3) |

Still shape (B) — **with the spine replaced**, now argued from data-loss rather than memory.

---

## C) STRONGEST CASE NOBODY ELSE MADE

**The ref's value is not that it is faster. It is that it can be given a release profile JSC cannot
have — and that property, not throughput, is what a zero-swap cgroup needs.**

Three converging facts:

1. `docker-compose.yml:6-7` — the entrypoint **auto-restarts opencode after a cgroup OOM-kill.** That
   loop **suppresses the evidence** needed to size the real problem. Every memory figure on this team
   comes from data the operator's own config is hiding.
2. perf §1b/§5 identify **JSC not returning peak heap** — under zero swap, RSS ratchets and never
   recovers. He marks this *asserted, not profiled* (his #5) and it is **the** mechanism that turns a
   transient spike into a permanent ratchet.
3. The ref has no `#[global_allocator]` and no release tuning — but it **can** have mimalloc with
   `MIMALLOC_ARENA_EAGER_COMMIT=0`, which is precisely "return pages to the OS."

**Narrower than any claim made this round, and the only version I can defend without a measurement.**
I note plugin-host measured IPC latency at 0.097-6.87 ms and the hook host at O(1) ~106 MB — those
objections are dead, and I am not reviving them.

---

## D) VERDICT

**On the tension: neither. False dichotomy, category error over three pools.** skeptic right on
priority (the two real defects are language-independent and come first); his magnitudes wrong twice
(54-server multiplier §B.4, and a 1200 MiB+swap container standing in for 8192 MiB zero-swap). perf
right on mechanism, wrong on pool (§A.2). **I now think the thing that matters most is neither thesis.**

**On adoption: the ref is worth taking for provider streaming, tools, and schema. It is not a drop-in** —
its persistence layer would convert existing DB corruption into total data loss, and its plugin and agent
layers had never been exercised against real inputs — **now partly falsified** (plugin-host loaded real
omo); the agent-resolution gap (3 vs 46 personas) is the part that survives.

**On the rewrite decision: nobody may responsibly recommend one yet — including me.** Two things must
happen first, in this order:

1. **Restore the database. It is a 10-minute fix and it is an availability incident, not a planning
   input.** I verified skeptic's recovery claim myself this turn:

   ```
   opencode.db         1,405,136,896 B  (Oct  5)  quick_check → FAIL, code 11
   opencode.fixed      1,349,709,824 B  (Sep 23)  quick_check → ok
   opencode.db.corrupt   954,097,664 B  (Sep 15)  quick_check → FAIL, code 11
   opencode.db.new              0 B  (Sep 15)
   ```
   On the clean file: **sessions=1,019 · messages=20,667 · parts=95,739 · events=0.**
   **A verified-clean DB has been sitting beside the corrupt one since Sep 23 and was never renamed in.
   The corrupt file grew 55 MB in the meantime.** Note `events=0` — consistent with §A.2.

   (My first `ls *.db*` missed `opencode.fixed` because the name contains no `.db`; corrected by full
   listing. Flagging it because I nearly reported a false refutation of a teammate.)

2. **Start the container and measure.** `docker stats` + `/proc` + a heap snapshot at the point of
   stall. Then re-ask the question.

**If the stall survives on a clean DB, in one directory, with 3 MCP servers, the rewrite question is
live. If it evaporates, this three-round exercise was answering a corruption artifact.**

---

## CLAIMS I COULD NOT VERIFY

1. **Whether `event` is truly absent from the fleet DB or merely unreadable-because-corrupt.** My
   evidence is absence from `dbstat` while `session` reads cleanly, plus `events=0` on `opencode.fixed`.
   Suggestive, not proof. **This matters most: if `event` exists and is only corrupt, perf's ~15 GiB win
   partially revives for this pool.** `.recover` would settle it; I did not run one (it writes).
2. **When the corruption occurred.** `-wal` is 0 B, `opencode.db` mtime is Oct 5 00:27, newest
   `session.time_updated` is **2026-09-19**. I cannot date the failure. skeptic dates it before Sep 15.
3. **Whether the corrupt b-tree is `message` (skeptic, rootpage 25) or `part` (my read: `part` failed
   too).** I saw `Tree 25` and `Tree 60` fail and `part` + `event` both error while `session` read
   clean; I did not map rootpages against the clean file. **skeptic's rootpage mapping is the better
   evidence and I defer to it.**
4. **Whether `elia-subworker-srv` has ever run with the current 1.31 GiB DB.** Absent from `docker ps`;
   DB written 00:27 today, so it may have run and been stopped.
5. **Per-directory MCP count for the fleet.** Bounded by 3 global entries plus per-workspace
   `opencode.json` files I did not audit individually (§B.4).
6. **Any RSS measurement, on any pool, by anyone.** §A is structural and file-level only.
7. **What causes the corruption** — concurrency pattern, filesystem, or an external writer. I identified
   the symptom class, not the cause.
8. **Whether restoring `opencode.fixed` loses the 55 MB of growth** accrued since Sep 23. I did not
   diff the two files' contents.
9. **Whether the `lost_and_found` table (6 MiB, present in fleet `dbstat`) indicates earlier repair
   attempts.** Not investigated.

---

# ROUND 3 — CONCESSIONS / POSITION / PLAN REQUIREMENTS

## 1. CONCESSIONS

**C1. My `readExact` mechanism was WRONG — plugin-host measured it, I asserted it.** I claimed
`"readable"` "never fires" under bun. **It fires** — on bun *and* deno, identically with and without
`setEncoding` (which answers my open question). Real cause is a **lost-wakeup race**:
`plugin-host.ts:96-103` calls `read()` first, attaching `once("readable")` only after `null`; an event in
between is lost, and `readMessage()`'s byte-by-byte `readExact(1)` widens the window. **Intermittent, not
deterministic.** Withdrawn.

**C2. "The host has likely never completed a handshake with any plugin" — WITHDRAWN as too strong.**
The pristine host works; my TIMEOUT was a cold-start artifact of my harness. My §B.3 rested on this.

**C3. My "4th spine item" claim about `hook_name_to_event` was WRONG.** It is off the critical path
(`client.rs:215-226` sends the hook name as a string) — new hooks need no new variant. **Three items drop
off my fix list.** Surviving: the `event` gap (a closed enum cannot express raw passthrough) and `-32600`.

**C4. Conceded to plugin-host's sharper version:** missing are `event` (serious), `config` (a *mutation*
hook — my `ConfigLoaded` is semantically wrong), `dispose`, `experimental.compaction.autocontinue`. Live
wire result is **12 hooks + 26 tools, not 14**; `:422` filters `event` and `config` — precisely the
ulw-loop state machine and the config mutation hook.

**C5.** My §1.7a magnitude stays withdrawn (I measured omo's event emissions, not subscriptions).

**C6. "Keep the crates, the remainder is cheap" — withdrawn.** perf §8 was right: I admired the loop's
fidelity without noticing it reproduces the RAM anti-pattern.

**C7. "event is absent from the fleet DB" — weakened to UNPROVEN.** Absence from `dbstat` is also what a
corrupt b-tree produces. `opencode.fixed` reports **events=0** — supportive, not conclusive. **If `event`
exists and is only corrupt, perf's 15 GiB reclaim partially revives here.**

**NOT withdrawn** (no rebuttal received): skeptic's **54-server multiplier is wrong** — his cross-attack
still prices it at `:135-136,197,301`; opencode reads `cfg.mcp` only, 3 entries. And **W1**:
`server.rs:197` full-resync `DELETE`+re-`INSERT` is a data-loss vector on this DB.

## 2. FINAL POSITION: **(b), heavily qualified and deferred**

Not (c), but (b) with the spine replaced **and gated behind measurement**. Provider-streaming, tool and
SQL layers are expensive to re-derive and genuinely not the bottleneck; persistence must be replaced
(W1), so **not a drop-in**. But (c)-forever is also wrong: a **corrupt `message` b-tree on every turn's hot
path**, plus a zero-swap cgroup where JSC's non-returning heap turns transient spikes into permanent
ratchets, are real defects config does not fix. Honest answer: **"neither (a), (b) nor (c) as framed"** —
the decision is unavailable until measured.

## 3. WHAT THE PLAN MUST CONTAIN

**Ordering constraint — the gate is a *measurement*, not a date:**
`restore DB -> start container -> measure at stall -> decide`, and **no porting code before step 4 passes.**
The plan must force:

1. **Is the stall reproducible on a clean DB, single directory, 3 MCP servers?** If not → we answered a
   corruption artifact; **stop.**
2. **Is `event` absent or corrupt?** `.recover`, not `dbstat`.
3. **RSS at N sessions on the real deployment?** Never measured, on any pool.
4. **Is `/instance/dispose` a 404 in production too, or Python-only?** Decides whether eviction is a Rust
   or a scheduler change.
5. **Which pool does the 8 GB actually hold?**

**What I will NOT let skipped:** the DB restore (10-minute fix, availability incident); a measured
baseline before any rewrite commitment; the W1 persistence replacement; and the `event`/`config` gap scoped
as *silent no-op*, not cosmetic.

---

# ROUND 4 — SELF-ERRORS / PLAN CHANGE / ARGUMENT AGAINST MYSELF

*(plugin-host flagged my file "truncated at §3". Not truncated — A/B/C/D + CLAIMS + Rounds 3 all
present. Stale copy.)*

## 1. THREE WAYS MY OWN ANALYSIS WAS WRONG

**E1. I diagnosed the memory blocker with the exact category error I charged perf-architect with.** My
§1.2 called unbounded `SessionManager` "the real blocker" for 100 sessions/8 GB. My own §A.2 then
measured the fleet's **entire** message history at **11 MiB across 1,019 sessions**. An 83.5 MiB
worst-case session cannot exist in a pool whose history is 11 MiB. **I held a severity calibrated to the
host TUI pool while lecturing the team about pool discipline.** The unbounded-RAM defect is real and must
still be fixed; its severity as *this project's* blocker was unsupported.

**E2. I published a mechanism from n=1.** C1 was a pattern: one observation plus a plausible reading,
promoted to a finding the lead quoted. Three "verified" claims rested on single probes — the `readable`
mechanism, the persona-loader root cause (two wrong guesses before I stopped), the bun-timeout
attribution. **I labelled the report evidence-based while three load-bearing cells were one point each.**

**E3. I never checked whether the target existed.** Three rounds in, I never ran `docker ps` — until
Round 3, where the deployment I was costing was **not running** and its database was **corrupt**. I
audited a system's *code* while its *actual failure* sat unrestored.

## 2. THE PLAN CHANGE THIS FORCES

**My §5 seven-phase table had the wrong first row.** It opened with Phase 0 = "add CI, scrub `unwrap`" —
hygiene on a codebase nobody has confirmed is the right target. **Phase 0 must be the measurement gate,
and it gates the port, not merely the plan:**

> **P0 (blocking, ~1 day):** swap in the clean `opencode.fixed`; start `elia-subworker-srv`; capture
> `docker stats` + `/proc` at the stall; `.recover` to settle absent-vs-corrupt `event`; RSS at N
> sessions. **No porting work — including Phase 0 CI — before P0 returns.**

A CI pass on a ref nobody ships is wasted motion, and I was about to schedule exactly that.

## 3. THE STRONGEST ARGUMENT AGAINST MY OWN CONCLUSION

**My §D "nobody may recommend a rewrite yet, including me" is safe, and safety is not free.**

1. **It defers a real win.** 25 agents keep running on a JSC heap that perf §1b shows does not return
   pages, under zero swap, with `docker-compose.yml:6-7` auto-restarting on OOM. That loop is not
   neutral: it masks degradation and costs unsaved session state on every kill. "Measure first" is
   right; "measure first, then defer indefinitely" reproduces the stall a season later.
2. **Interest bias.** I spent three rounds auditing the ref and none auditing the container, so my "the
   two cheap fixes may suffice" is my *least-verified* claim — promoted in §D as if measured. Both "not a
   drop-in" and "config might be enough" are claims dressed as findings.
3. **The pro-port argument I under-weighted:** the ref's per-step full history reload
   (`prompt.rs:954-960`) and 85 tool-schema deep-clones are CPU costs pooling cannot touch, and they sit
   on a path with a **verified corrupt b-tree** — where the fix is a storage rewrite regardless of
   language. **On the corrupt `message` table, a Rust persistence layer is required even if every other
   motive evaporates.**

**Position moves one step:** not "(b), deferred" but **"(b) is the default; P0 is a two-day gate that can
cancel it; if P0 says config-only, ship those fixes and revisit — do not keep waiting."**

---

# ROUND 5 — THE MEASUREMENT FAILURE BEHIND MY OWN RETRACTION

## 1. THREE PROCESS FAILURES (not conclusion failures — process)

**F1. E2 recurred, on my most confident claim.** Round 4's E2 named the pattern: publishing an inference as
a finding. §A.2 did it — `event` "absent from `dbstat`" became "does not exist," and I used it to strike
perf's #1 finding. **My own second query falsified it:** `SUM(pgsize) FROM dbstat` = 498 MiB vs
`page_count × page_size` = 1340 MiB — **dbstat accounts for 37% of the file.** An alphabetical walk lists
`event` at 0 MiB *and* its three indexes at 53 MiB; **an index cannot exist without its table**, so the
table exists and the scan stopped reporting its pages. **dbstat aborts mid-scan on a corrupt DB, making
every per-table figure it produces unreliable here.** Withdrawn.

**F2. Unverified-claim drift.** I carried "is `event` absent or corrupt?" as my #1 unverified claim for
three rounds *while* publishing a confident answer to it. **A claim I cannot verify and a conclusion I have
drawn cannot both be load-bearing.** When they conflict the conclusion loses. It did.

**F3. I used a convenience method where a correctness method existed, and never said so.** `dbstat` is one
query; `.recover` reads b-tree pages directly. **Both my "perf measured the wrong pool" attack and my own
rebuttal rested on a method I knew was questionable.** A corrupt-DB measurement technique is a caveat, not
a footnote.

**Net effect:** perf's ~15 GiB claim is **UNMEASURED here, not refuted** — I withdraw that part of §A.2.
The pool-level point survives on `page_count` alone. ts-anatomy reached the opposite conclusion by a
different route (`COUNT(*)` fails = corrupt-but-present), so **two of us agree the table exists and neither
has sized it.** Anyone quoting my "absent from `dbstat`" line should discard it.

## 2. PLAN CHANGE

**Round 4's P0 reorder stands and gains a step.** §5 still opens with "add CI, scrub `unwrap`" — hygiene
on an unconfirmed target, now also built on a per-table measurement I have retracted.

> **P0 (blocking, ~1 day):** (1) swap in the clean `opencode.fixed`; (2) **`.recover` — never `dbstat`** —
> settle `event` absent-vs-corrupt *and size it*; (3) start `elia-subworker-srv`; (4) `docker stats` +
> `/proc` at the stall; (5) RSS at N sessions. **No porting work, CI included, before P0 returns.**

## 3. STRONGEST ARGUMENT AGAINST MYSELF

**A method error is worse than a wrong opinion: it silently launders everyone else's numbers too.**

perf's `dbstat` figures came from a **healthy** 17.69 GiB DB where the method works — his work is sound and
I should not generalise my failure onto it. But the reverse hazard is mine: **every per-table number on
this team derived from the container DB is now suspect**, including parts of my own §A.2 and whatever
skeptic and perf drew from it. We have spent three rounds arguing quantitatively about a database whose
`integrity_check` fails, and only two of us noticed.

**The honest statement: this team has no reliable per-table accounting of the fleet database at all.** A
team that argues this long from an unmeasured, corrupt substrate will produce confident artefacts.
`.recover` first. Everything else is downstream.
