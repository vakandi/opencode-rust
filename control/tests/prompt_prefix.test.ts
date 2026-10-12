/**
 * prompt_prefix.test.ts — the prefix must be byte-identical to what is on disk.
 *
 * The trap this guards: an agent's PROMPT.md tail differs per agent
 * (21/22 carry the EliaAI sentence, linkedin-connector-ch-lu-fr does not). A
 * generator that "normalises" the prefix silently rewrites that agent's warning.
 * So the assertion is per-file round-trip, not a shared golden string.
 */
import { test, expect } from "bun:test";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";
import {
  prefixFor,
  homeFor,
  workspaceFor,
  rewriteAnchors,
  tailOf,
  isOmpHomeLine,
  withPrefix,
  OLD_REPO_ROOT,
  TAILS,
} from "../promptPrefix.ts";

// This test file lives at <repo>/control/tests/, so the repo root is two levels up.
const REPO_ROOT = new URL("../..", import.meta.url).pathname.replace(/\/+$/, "");
const SUBWORKERS = join(REPO_ROOT, "subworkers");

const names = readdirSync(SUBWORKERS, { withFileTypes: true })
  .filter((e) => e.isDirectory() && existsSync(join(SUBWORKERS, e.name, "PROMPT.md")))
  .map((e) => e.name);

test("prefix round-trips byte-identical across every on-disk PROMPT.md", () => {
  expect(names.length).toBeGreaterThanOrEqual(22);

  let anchor2Count = 0;
  for (const name of names) {
    const path = join(SUBWORKERS, name, "PROMPT.md");
    const original = readFileSync(path, "utf8");
    const lines = original.split("\n");

    // Anchor 1 must exist on line 2 for every agent.
    expect(isOmpHomeLine(lines[1] ?? "")).toBe(true);

    // The generator reproduces this file's OWN line 2, tail included.
    const generated = prefixFor(name, REPO_ROOT, tailOf(lines[1] ?? "") ?? TAILS.CANONICAL);
    const rewrittenLine2 = rewriteAnchors(original, name, REPO_ROOT).split("\n")[1] ?? "";
    expect(rewrittenLine2).toBe(generated);

    // Anchor 2: the workspace path must move too.
    if (original.includes("## Workspace Constraint")) {
      anchor2Count++;
      const rewritten = rewriteAnchors(original, name, REPO_ROOT);
      expect(rewritten).toContain("`" + workspaceFor(name, REPO_ROOT) + "`");
    }
  }

  // We assert the real disk state, not an assumption.
  expect(anchor2Count).toBeGreaterThanOrEqual(15);
});

test("rewrite leaves zero omp-server paths outside the workspace dir", () => {
  for (const name of names) {
    const original = readFileSync(join(SUBWORKERS, name, "PROMPT.md"), "utf8");
    const rewritten = rewriteAnchors(original, name, REPO_ROOT);
    // Anchor 3 (self-reference in SOURCE DE VERITE) must move too.
    expect(rewritten).not.toContain(OLD_REPO_ROOT + "/subworkers/" + name);
    expect(rewritten).toContain(homeFor(name, REPO_ROOT));
  }
});

test("rewrite never touches a sibling agent's home or the container path", () => {
  const original = readFileSync(join(SUBWORKERS, "cve-hunter", "PROMPT.md"), "utf8");
  const rewritten = rewriteAnchors(original, "cve-hunter", REPO_ROOT);
  expect(rewritten).not.toContain(homeFor("test-delegator", OLD_REPO_ROOT));
  expect(rewritten).toContain("/Users/vakandi/EliaAI/context/TOOLS.md");

  // The container path is the in-container location of the same folder. Asserted on
  // a file that actually carries one rather than assumed to be universal.
  const withContainer = names.find((n) =>
    readFileSync(join(SUBWORKERS, n, "PROMPT.md"), "utf8").includes("/data/subworkers/" + n + "/workspace/"),
  );
  expect(withContainer).toBeDefined();
  const kept = rewriteAnchors(
    readFileSync(join(SUBWORKERS, withContainer as string, "PROMPT.md"), "utf8"),
    withContainer as string,
    REPO_ROOT,
  );
  expect(kept).toContain("/data/subworkers/" + withContainer + "/workspace/");
});

test("per-agent tails are preserved, never normalised across agents", () => {
  const delegator = readFileSync(join(SUBWORKERS, "test-delegator", "PROMPT.md"), "utf8");
  const shortName = names.find((n) => {
    const line = readFileSync(join(SUBWORKERS, n, "PROMPT.md"), "utf8").split("\n")[1] ?? "";
    return !(tailOf(line) ?? "").includes("EliaAI/ paths in new work");
  });
  expect(shortName).toBeDefined();
  const shortPrompt = readFileSync(join(SUBWORKERS, shortName as string, "PROMPT.md"), "utf8");

  expect(tailOf(delegator.split("\n")[1] ?? "")).toBe(TAILS.CANONICAL);
  expect(tailOf(shortPrompt.split("\n")[1] ?? "")).toBe(TAILS.SHORT);

  const a = rewriteAnchors(delegator, "test-delegator", REPO_ROOT).split("\n")[1] ?? "";
  const b = rewriteAnchors(shortPrompt, shortName as string, REPO_ROOT).split("\n")[1] ?? "";
  expect(tailOf(a)).toBe(TAILS.CANONICAL);
  expect(tailOf(b)).toBe(TAILS.SHORT);
});

test("refund-hunter's 64 KB PROMPT is never truncated", () => {
  const name = "refund-hunter";
  if (!existsSync(join(SUBWORKERS, name, "PROMPT.md"))) return;
  const original = readFileSync(join(SUBWORKERS, name, "PROMPT.md"), "utf8");
  // Guard against the 12k cap that exists in the old worker.ts: the rewrite must
  // return a body at least as long as the original minus the path delta.
  const rewritten = rewriteAnchors(original, name, REPO_ROOT);
  expect(rewritten.length).toBeGreaterThan(original.length - 200);
  expect(rewritten.length).toBeGreaterThan(10_000);
});

test("withPrefix builds a block for an agent with no on-disk anchor", () => {
  const out = withPrefix("BODY", "brand-new-agent", REPO_ROOT);
  expect(out.split("\n")[0]).toBe(prefixFor("brand-new-agent", REPO_ROOT));
  expect(out).toContain("`" + workspaceFor("brand-new-agent", REPO_ROOT) + "`");
  expect(out.endsWith("BODY")).toBe(true);
});