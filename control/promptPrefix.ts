/**
 * promptPrefix.ts — per-agent containment prefix, generated VERBATIM.
 *
 * There is no templating. HOWTOADDAGENTS.md states injection is verbatim, and the
 * 22 PROMPT.md files on disk are the source of truth. So this module does exactly
 * one thing per anchor: swap the absolute repo path. Every other byte of the line
 * (including the per-agent tail, which differs between agents) is preserved.
 *
 * TWO anchors, both must move or the agent writes outside its sandbox:
 *   1. line 2 — `> OMP HOME: <home> — ...`      the agent's home directory
 *   2. `## Workspace Constraint` block — `<home>/workspace/`  the only writable dir
 *
 * A third path appears inside that block: the container runtime path
 * `/data/subworkers/<name>/workspace/`. It is NOT rewritten — it is the in-container
 * location of the same folder and rewriting it would break Docker-style mounts.
 */

/** Line-2 anchor head. Everything after the path is per-agent and preserved. */
const OMP_HOME_HEAD = "> OMP HOME: ";

/** The workspace constraint block head. */
const WORKSPACE_HEAD = "## Workspace Constraint";

/** Longest tail seen on disk (21/22 agents). Kept as the canonical default. */
const CANONICAL_TAIL =
  " — all reads/writes stay inside this home (plus its workspace/ subdir). " +
  "Never use /Users/vakandi/EliaAI/ paths in new work; old outputs keep working via the live bind-mount.";

/** Short tail used by linkedin-connector-ch-lu-fr. Preserved verbatim, never normalised. */
const SHORT_TAIL = " — all reads/writes stay inside this home (plus its workspace/ subdir).";

/**
 * The repo root the anchors were written against. PROMPT.md files on disk still
 * carry it, and `rewriteHomePaths` needs it as the search key. Exported so tests can
 * assert the absence of old paths without hardcoding the string a second time.
 */
export const OLD_REPO_ROOT = "/Users/vakandi/Documents/omp-server";

/** Absolute home directory for an agent, always with a trailing slash. */
export function homeFor(name: string, repoRoot: string): string {
  return `${repoRoot.replace(/\/+$/, "")}/subworkers/${name}/`;
}

/** Absolute workspace directory for an agent, always with a trailing slash. */
export function workspaceFor(name: string, repoRoot: string): string {
  return `${homeFor(name, repoRoot)}workspace/`;
}

/**
 * The canonical line-2 prefix for an agent.
 * `tail` defaults to the on-disk majority form; pass SHORT_TAIL-shaped text only
 * when replacing inside a file that already uses it.
 */
export function prefixFor(name: string, repoRoot: string, tail: string = CANONICAL_TAIL): string {
  return `${OMP_HOME_HEAD}${homeFor(name, repoRoot)}${tail}`;
}

/** True when `line` is the line-2 OMP HOME anchor. */
export function isOmpHomeLine(line: string): boolean {
  return line.startsWith(OMP_HOME_HEAD);
}

/**
 * Extract the tail of an existing OMP HOME line (everything after the path), so a
 * rewrite can put it back byte for byte. Returns null when the line is not an anchor.
 *
 * The path is whatever sits between "> OMP HOME: " and the first " — " that follows
 * a trailing-slash directory. Trailing slash is the reliable terminator because
 * every home path on disk ends in one and no tail contains " — " before it.
 */
export function tailOf(line: string): string | null {
  if (!isOmpHomeLine(line)) return null;
  const rest = line.slice(OMP_HOME_HEAD.length);
  const end = rest.lastIndexOf("/ — ");
  if (end < 0) return null;
  return rest.slice(end + 1);
}

/**
 * Rewrite every absolute path that points at THIS agent's own home, whichever
 * anchor carries it.
 *
 * The disk reality is THREE anchors, not two. `rewriteAnchors`'s line-2 + workspace
 * pass covers the two the plan documented, but a number of PROMPT.md files also
 * self-reference their own path in the SOURCE DE VÉRITÉ read-order block
 * (e.g. cve-hunter.md:52 points at `.../subworkers/cve-hunter/PROMPT.md`). Left
 * alone, that is a read instruction aimed at the OLD repo — the agent would go
 * there for its own instructions on every run.
 *
 * Scoped by exact home path, so it can never touch another agent's home, a
 * sibling agent's workspace, or the /data container paths.
 */
export function rewriteHomePaths(text: string, name: string, repoRoot: string): string {
  const oldHome = homeFor(name, OLD_REPO_ROOT);
  return text.split(oldHome).join(homeFor(name, repoRoot));
}

/**
 * Rewrite both anchors in a PROMPT.md body to point at `repoRoot`.
 *
 * Returns the text unchanged when an anchor is absent — a missing anchor is a fact
 * to surface in a report, never something to paper over by inventing a line.
 */
export function rewriteAnchors(text: string, name: string, repoRoot: string): string {
  const home = homeFor(name, repoRoot);
  const workspace = workspaceFor(name, repoRoot);
  const scoped = rewriteHomePaths(text, name, repoRoot);
  const out: string[] = [];

  for (const line of scoped.split("\n")) {
    if (isOmpHomeLine(line)) {
      const tail = tailOf(line);
      // Preserve this file's own tail verbatim; never normalise across agents.
      out.push(tail === null ? `${OMP_HOME_HEAD}${home}${CANONICAL_TAIL}` : `${OMP_HOME_HEAD}${home}${tail}`);
      continue;
    }
    out.push(line);
  }

  const rewritten = out.join("\n");
  // Anchor 2: the workspace path on its own backticked line inside the block.
  // `/data/subworkers/...` is the in-container location of the same folder and must
  // survive untouched — replacing it would break any bind mount and lie to the agent
  // about where its files actually are.
  return rewritten.replace(
    /`(\/(?!data\/)[^`\n]*\/subworkers\/[^`\n]*\/workspace\/)`/g,
    (_match, path: string) =>
      path.includes("/subworkers/" + name + "/") ? "`" + workspace + "`" : "`" + path + "`",
  );
}

/**
 * Prepend the prefix when the caller has no PROMPT.md to rewrite. Used for agents
 * imported later that have no on-disk anchor yet.
 */
export function withPrefix(promptBody: string, name: string, repoRoot: string, tail: string = CANONICAL_TAIL): string {
  const block = [
    prefixFor(name, repoRoot, tail),
    "",
    WORKSPACE_HEAD,
    "You MUST only read and write files inside your `workspace/` folder:",
    "`" + workspaceFor(name, repoRoot) + "`",
    "Never write files outside this folder. All system paths outside workspace/ are blocked.",
  ].join("\n");
  return `${block}\n\n${promptBody}`;
}

export const TAILS = { CANONICAL: CANONICAL_TAIL, SHORT: SHORT_TAIL } as const;