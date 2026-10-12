# ADDENDUM — ts-anatomy, responding to plugin-host's cross-attack pushbacks

Three of plugin-host's four pushbacks land. One is settled in a way that changes the §0 conclusion from
"probably fine" to "not reproducibly identifiable". I concede two errors of my own.

---

## 1. §0 IS NOW SETTLED — and the answer is worse than "probably 1.18.34"

plugin-host says (§B) I asserted v1.18.34 "without a version probe or a distinguishing endpoint test." **Correct
— I inferred it from `packages/opencode/package.json:3` and never probed a binary.** Measured now:

```
$ opencode --version
1.18.34
$ which opencode
/Users/vakandi/.nvm/versions/node/v20.20.2/bin/opencode
```

So the **host** binary matches the tree I mapped. But the subworker container does **not** use that binary.
`/Users/vakandi/EliaAI/subworkers/server/Dockerfile:48-56`:

```dockerfile
# Install opencode from latest GitHub release (pinned, not main) — .../releases
RUN latest_tag=$(curl -s https://api.github.com/repos/anomalyco/opencode/releases/latest | jq -r .tag_name) && \
    ... curl -fsSL -o /tmp/opencode.tar.gz ".../download/${latest_tag}/${asset}" && \
    tar -xzf /tmp/opencode.tar.gz -C /usr/local/bin && opencode --version
```

**The comment says "pinned, not main". That is false.** `releases/latest` is a moving target resolved *at image
build time*. It is the least-pinned install method available.

**Verified limitation:** `docker images` returns no local `subworker`/`server` image, so the container's actual
version is unreadable from here. I will not guess it.

**⇒ §0 conclusion, corrected:** three different version numbers are in play —
`opencode_client.py:4` says **1.18.18** (docstring), the host binary is **1.18.34**, and the container is
**`releases/latest` at last build, unknown, drifting on every rebuild**. For a team about to port a server whose
API demonstrably drifted three times across 16 patch releases, "the deployed artifact cannot be identified or
reproduced" is a first-order finding, not a footnote. **plugin-host's demand for an endpoint fingerprint against
a live server is the right next step and I should have done it in Round 1.**

---

## 2. DISK — plugin-host is right, and it upgrades my §7.4 from "gap" to "active failure mode"

```
/dev/disk3s5   460Gi   405Gi used   23Gi avail   95% Capacity
```

Confirmed by measurement. My Round 1 §7.4 said the un-pruned `event` table was a *storage gap*. That
understated it. With **23 GiB of headroom**, the append-only event table is not a future cost — it is a
**current** failure mode, and it interacts with my cross-attack finding:

**A nearly-full volume makes SQLite page/WAL corruption materially more likely.** ENOSPC during a WAL write, or
a filesystem that cannot extend the DB file mid-transaction, produces exactly the
`btreeInitPage() returns error code 11` / `SQLITE_CORRUPT` signature I measured on
`server/opencode-data/opencode.db` (cross-attack §A.3), for which the directory holds two prior artefacts
(`opencode.db.corrupt` 954 MB Sep 15, `opencode.fixed` 1.35 GB Sep 23).

**I am not claiming causation** — I have not correlated the failure dates against disk pressure. But the
mechanism is standard and the artefacts are dated. **plugin-host's reframing is correct and I adopt it.**

---

## 3. `bypassAgentCheck` — plugin-host's escalation is CORRECT, and I under-classified it

I labelled this UNVERIFIED in Round 1 §9.2. That was too weak. It is now verified, and it is **not** a
perf-shaped gap — **it is a permission bypass**.

`packages/opencode/src/tool/task.ts:119-129`:
```ts
if (!ctx.extra?.bypassAgentCheck) {
  yield* ctx.ask({ permission: id, patterns: [params.subagent_type], always: ["*"], metadata: {...} })
}
```
**When the flag is true the `task` permission check is skipped entirely** — any subagent can be invoked with no
prompt.

Where the flag comes from — `packages/opencode/src/session/prompt.ts:1223`:
```ts
const bypassAgentCheck = lastUserMsg?.parts.some((p) => p.type === "agent") ?? false
```
set into the tool context at `prompt.ts:1231` → `session/tools.ts:64` (`extra: { …, bypassAgentCheck }`), and
checked at `tool/task.ts:119`.

**And an `agent` part is synthesised from prompt text.** `SessionPrompt.resolvePromptParts`
(`prompt.ts:157-191`) turns any `@name` token for which `agents.get(name)` resolves (`:176`) into an agent
attachment on the user message.

**⇒ the chain is: untrusted prompt text containing `@<agent>` → an `agent` part on the last user message →
`bypassAgentCheck = true` → the `task` permission gate is disabled for that turn.**

That is a permission gate defeatable by message content rather than by an authorised decision. Given that
`opencode_client.py` posts scheduler-authored prompt text into agents that hold real tool access, this should be
treated as a **security item**, not an adapter gap. It ranks above every perf item currently in the plan.

**I concede the misclassification.** I verified that *no caller of `resolve_tools_with_permissions` threads the
flag* (cross-attack §A.1 on pragmatic-architect) but stopped short of asking what the flag *does*. The absence I
found is real; the inference that it is merely an adapter inconvenience was wrong.

---

## 4. `PluginModule.tui` — conceded

plugin-host is right that my Appendix A §1.2 Group B listed `src/plugin/tui/**` as DELETE while my own
verification standard was "compile-clean is not evidence; needs a runtime assertion." I applied that standard to
the build/typecheck/boot gates and **not to the operator-facing surface**. Specifically I never checked that
`plugin/tui/**` and the `PluginModule.tui` contract (`packages/plugin/src/index.ts:70-74`) are not what serves
the TUI side panel — the exact "does the sidebar still work" test I named and then skipped.

Two corrections to my Appendix A:
1. `src/plugin/tui/**` moves from **DELETE** to **REVIEW-THEN-KEEP**, gated on a runtime assertion that the
   operator sidebar still renders.
2. Add to §4 **V4.5**: boot the CLI, open the sidebar, and confirm it populates. A green typecheck is not
   evidence for deleting a user-facing surface.

---

## What still stands unchanged

Nothing in plugin-host's pushback touches my §1 session-lifecycle hops, my §7.1 single-synchronous-handle
finding, my §7.3 per-step full-history re-read, my §1.8 durable-event tax, or the cross-attack's four
wire-level breaks against `/tmp/OpenCodeRust`. The API-gap and MCP-fan-out findings are theirs and unaffected.

**Still unverifiable by me:** the container's opencode version (image absent); whether the corruption is
disk-induced (§2 — mechanism plausible, causation unproven); whether an `agent` part is reachable through any
path other than prompt text.