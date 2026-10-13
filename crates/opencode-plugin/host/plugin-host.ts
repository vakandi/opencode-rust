/**
 * plugin-host.ts — JSON-RPC host for a single TypeScript plugin.
 *
 * Embedded into the Rust binary via include_str!() and written to
 * ~/.cache/opencode/plugin-host.ts at runtime.
 *
 * Protocol: Content-Length framed JSON-RPC 2.0 over stdin/stdout.
 * stderr is reserved for plugin log output.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
}

interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

interface PluginContext {
  worktree: string;
  directory: string;
  serverUrl: string;
}

type UnknownRecord = Record<string, unknown>;

interface Hooks {
  [key: string]: ((input: unknown, output: unknown) => Promise<unknown>) | unknown;
}

interface ToolLike {
  description?: string;
  args?: unknown;
  execute: (args: unknown, context: unknown) => Promise<unknown>;
}

interface ZodLike {
  _zod?: { toJSONSchema?: () => UnknownRecord };
  description?: string;
}

function jsonSchemaFromArgs(args: unknown): UnknownRecord {
  const properties: UnknownRecord = {};
  const required: string[] = [];

  if (args && typeof args === "object") {
    for (const [paramName, maybeSchema] of Object.entries(args as UnknownRecord)) {
      const schema = maybeSchema as ZodLike | undefined;
      const convert = schema?._zod?.toJSONSchema;
      if (typeof convert !== "function") continue;

      let converted: UnknownRecord;
      try {
        converted = convert.call(schema?._zod);
      } catch {
        continue;
      }
      if (!converted || typeof converted !== "object") continue;

      const { $schema: _ignored, ...rest } = converted;
      properties[paramName] = rest;
      if (rest.type !== "object" || !("default" in rest)) {
        if (!rest.nullable && rest.type !== "null") required.push(paramName);
      }
      if (typeof schema?.description === "string" && rest.description === undefined) {
        properties[paramName] = { ...rest, description: schema.description };
      }
    }
  }

  return { type: "object", properties, required };
}

interface AuthMethod {
  type: string;
  label: string;
  inputs?: Record<string, { placeholder?: string; required?: boolean }>;
}

interface AuthorizeResult {
  url?: string;
  instructions?: string;
  method?: string;
  callback?: (code?: string) => Promise<unknown>;
}

interface AuthHook {
  provider: string;
  methods: AuthMethod[];
  authorize?: (method: AuthMethod, inputs?: Record<string, string>) => Promise<AuthorizeResult>;
  loader?: () => Promise<{
    apiKey?: string;
    fetch?: typeof globalThis.fetch;
    [key: string]: unknown;
  }>;
}

// ---------------------------------------------------------------------------
// Content-Length framing
// ---------------------------------------------------------------------------

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function encodeResponse(response: JsonRpcResponse): Uint8Array {
  const body = JSON.stringify(response);
  const header = `Content-Length: ${Buffer.byteLength(body)}\r\n\r\n`;
  return encoder.encode(header + body);
}

/**
 * Read exactly `n` bytes from stdin.
 */
async function readExact(n: number): Promise<Uint8Array> {
  const chunks: Uint8Array[] = [];
  let remaining = n;

  const reader = process.stdin as unknown as {
    read(size: number): Uint8Array | null;
    once(event: string, cb: () => void): void;
  };

  while (remaining > 0) {
    const chunk: Uint8Array | null = reader.read(remaining);
    if (chunk !== null) {
      chunks.push(chunk);
      remaining -= chunk.length;
    } else {
      await new Promise<void>((resolve) => reader.once("readable", resolve));
    }
  }

  if (chunks.length === 1) return chunks[0];
  const result = new Uint8Array(n);
  let offset = 0;
  for (const c of chunks) {
    result.set(c, offset);
    offset += c.length;
  }
  return result;
}

/**
 * Read one Content-Length framed JSON-RPC message from stdin.
 * Returns null on EOF.
 */
async function readMessage(): Promise<JsonRpcRequest | null> {
  // Read header lines until empty line
  let header = "";
  while (true) {
    const byte = await readExact(1);
    if (byte.length === 0) return null;
    header += decoder.decode(byte);
    if (header.endsWith("\r\n\r\n")) break;
  }

  const match = header.match(/Content-Length:\s*(\d+)/i);
  if (!match) {
    throw new Error(`Invalid header: ${header}`);
  }

  const contentLength = parseInt(match[1], 10);
  const body = await readExact(contentLength);
  return JSON.parse(decoder.decode(body));
}

function send(response: JsonRpcResponse): void {
  process.stdout.write(encodeResponse(response));
}

function sendNotification(method: string, params?: unknown): void {
  const body = JSON.stringify({
    jsonrpc: "2.0",
    method,
    params,
  } satisfies JsonRpcNotification);
  const header = `Content-Length: ${Buffer.byteLength(body)}\r\n\r\n`;
  process.stdout.write(encoder.encode(header + body));
}

function sendResult(id: number, result: unknown): void {
  send({ jsonrpc: "2.0", id, result });
}

function sendError(id: number, code: number, message: string): void {
  send({ jsonrpc: "2.0", id, error: { code, message } });
}

// ---------------------------------------------------------------------------
// Plugin state
// ---------------------------------------------------------------------------

let pluginHooks: Hooks = {};
let authHook: AuthHook | null = null;
let pendingAuthCallback: ((code?: string) => Promise<unknown>) | null = null;
let customFetch: typeof globalThis.fetch | null = null;

// ---------------------------------------------------------------------------
// Plugin input compatibility helpers
// ---------------------------------------------------------------------------

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

function templateToShellCommand(parts: TemplateStringsArray, values: unknown[]): string {
  let command = parts[0] ?? "";
  for (let i = 0; i < values.length; i++) {
    const value = values[i];
    const serialized =
      typeof value === "string"
        ? value
        : typeof value === "number" || typeof value === "boolean"
          ? String(value)
          : JSON.stringify(value);
    command += shellQuote(serialized) + (parts[i + 1] ?? "");
  }
  return command;
}

async function runShellCommand(command: string): Promise<{
  stdout: string;
  stderr: string;
  exitCode: number;
}> {
  const childProcess = await import("node:child_process");
  return await new Promise((resolve, reject) => {
    const child = childProcess.spawn("bash", ["-lc", command], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk: Buffer | string) => {
      stdout += String(chunk);
    });
    child.stderr.on("data", (chunk: Buffer | string) => {
      stderr += String(chunk);
    });
    child.on("error", (err: unknown) => {
      reject(err);
    });
    child.on("close", (code: number | null) => {
      const exitCode = code ?? 1;
      const result = { stdout, stderr, exitCode };
      if (exitCode === 0) {
        resolve(result);
        return;
      }
      reject(new Error(stderr || `Command failed with exit code ${exitCode}`));
    });
  });
}

function createShell(): (parts: TemplateStringsArray, ...values: unknown[]) => Promise<unknown> {
  const maybeBun = (globalThis as UnknownRecord)["Bun"];
  if (maybeBun && typeof maybeBun === "object") {
    const dollar = (maybeBun as UnknownRecord)["$"];
    if (typeof dollar === "function") {
      return dollar as (parts: TemplateStringsArray, ...values: unknown[]) => Promise<unknown>;
    }
  }

  return async (parts: TemplateStringsArray, ...values: unknown[]): Promise<unknown> => {
    const command = templateToShellCommand(parts, values);
    return await runShellCommand(command);
  };
}

function createNoopClientProxy(path: string[] = []): unknown {
  const fn = async () => ({});
  return new Proxy(fn, {
    get(_target, prop: string | symbol) {
      if (typeof prop === "symbol") {
        if (prop === Symbol.toStringTag) return "PluginNoopClient";
        return undefined;
      }
      if (prop === "then") return undefined;
      if (prop === "toString") {
        return () => `[PluginNoopClient ${path.join(".")}]`;
      }
      return createNoopClientProxy([...path, prop]);
    },
    apply() {
      return Promise.resolve({});
    },
  });
}

async function createPluginClient(
  context: PluginContext,
  pluginPath: string,
): Promise<unknown> {
  const candidateUrls = new Set<string>();
  const addCandidate = (url: string) => {
    candidateUrls.add(url);
  };

  try {
    const sdk = (await import("@opencode-ai/sdk")) as UnknownRecord;
    const createOpencodeClient = sdk["createOpencodeClient"];
    if (typeof createOpencodeClient === "function") {
      return (createOpencodeClient as (config: UnknownRecord) => unknown)({
        baseUrl: context.serverUrl,
        directory: context.directory,
      });
    }
  } catch {
    // Fallback below.
  }

  try {
    const pathMod = await import("node:path");
    const urlMod = await import("node:url");

    const addCandidatePath = (path: string) => {
      addCandidate(urlMod.pathToFileURL(path).href);
    };

    let pluginFsPath: string | null = null;
    if (pluginPath.startsWith("file://")) {
      pluginFsPath = urlMod.fileURLToPath(pluginPath);
    } else if (pluginPath.startsWith("/")) {
      pluginFsPath = pluginPath;
    }

    addCandidatePath(
      pathMod.join(
        process.cwd(),
        "node_modules",
        "@opencode-ai",
        "sdk",
        "dist",
        "index.js",
      ),
    );

    if (pluginFsPath) {
      let cursor = pathMod.dirname(pluginFsPath);
      while (true) {
        addCandidatePath(
          pathMod.join(
            cursor,
            "node_modules",
            "@opencode-ai",
            "sdk",
            "dist",
            "index.js",
          ),
        );
        const parent = pathMod.dirname(cursor);
        if (parent === cursor) break;
        cursor = parent;
      }
    }

    try {
      const moduleMod = await import("node:module");
      const addFromRequire = (basePath: string) => {
        try {
          const req = moduleMod.createRequire(basePath);
          const resolved = req.resolve("@opencode-ai/sdk");
          addCandidate(urlMod.pathToFileURL(resolved).href);
        } catch {
          // Try next base path.
        }
      };
      addFromRequire(pathMod.join(process.cwd(), "package.json"));
      if (pluginFsPath) {
        addFromRequire(pluginFsPath);
        addFromRequire(pathMod.join(pathMod.dirname(pluginFsPath), "package.json"));
      }
    } catch {
      // createRequire path resolution is optional.
    }
  } catch {
    // If node:module/node:url is unavailable, keep noop fallback.
  }

  for (const url of candidateUrls) {
    try {
      const sdk = (await import(url)) as UnknownRecord;
      const createOpencodeClient = sdk["createOpencodeClient"];
      if (typeof createOpencodeClient === "function") {
        return (createOpencodeClient as (config: UnknownRecord) => unknown)({
          baseUrl: context.serverUrl,
          directory: context.directory,
        });
      }
    } catch {
      // Try next candidate.
    }
  }

  return createNoopClientProxy(["client"]);
}

function buildPluginInput(context: PluginContext, client: unknown): UnknownRecord {
  let serverUrl: string | URL = context.serverUrl;
  try {
    serverUrl = new URL(context.serverUrl);
  } catch {
    // Keep as string if URL parsing fails.
  }

  return {
    // Legacy Rust host shape (already used by some plugins)
    context,
    // TS plugin ecosystem shape
    client,
    directory: context.directory,
    worktree: context.worktree,
    serverUrl,
    project: {
      directory: context.directory,
      worktree: context.worktree,
    },
    $: createShell(),
  };
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async function handleInitialize(
  id: number,
  params: { pluginPath: string; context: PluginContext },
): Promise<void> {
  try {
    const mod = await import(params.pluginPath);
    const entry = mod.default ?? mod;
    // Accept the PluginModule v1 shape as well as a bare function. omo (oh-my-openagent)
    // exports `{ id, server }`, so `mod.default` is an object and a bare-function check
    // rejects the whole plugin with -32600. The TypeScript server resolves the same shapes via
    // `readV1Plugin` (packages/opencode/src/plugin/index.ts).
    const pluginFn =
      typeof entry === "function"
        ? entry
        : typeof entry?.server === "function"
          ? entry.server
          : undefined;

    if (typeof pluginFn !== "function") {
      sendError(id, -32600, "Plugin module does not export a function");
      return;
    }

    // Build a PluginInput compatible with upstream @opencode-ai/plugin shape,
    // while keeping `context` for backward compatibility.
    const client = await createPluginClient(params.context, params.pluginPath);
    const pluginInput = buildPluginInput(params.context, client);

    const hooks: Hooks = await pluginFn(pluginInput);
    pluginHooks = hooks;

    // Collect hook names
    const hookNames: string[] = [];
    for (const key of Object.keys(hooks)) {
      if (key === "auth") continue;
      if (typeof hooks[key] === "function") {
        hookNames.push(key);
      }
    }

    // Collect contributed tools. `hooks.tool` is a map of tool definitions, not a function,
    // so the loop above skips it entirely and the tools were silently dropped: omo's 14 tools
    // could never reach the LLM. Names alone are not enough to invoke a tool, so the
    // definition is forwarded for the host to register.
    //
    // A raw definition cannot be forwarded as-is: `args` holds live zod instances and
    // `execute` is a function, and JSON.stringify drops both, leaving a name with no schema
    // and no way to invoke it. omo attaches `_zod.toJSONSchema()` to every arg schema
    // (normalize-tool-arg-schemas.ts), so convert here, inside bun, where zod is live.
    const toolDefs: Record<string, unknown> = {};
    const toolHook = (hooks as { tool?: Record<string, unknown> }).tool;
    if (toolHook && typeof toolHook === "object") {
      for (const [toolName, rawDefinition] of Object.entries(toolHook)) {
        const definition = rawDefinition as ToolLike;
        toolDefs[toolName] = {
          description: typeof definition.description === "string" ? definition.description : "",
          parameters: jsonSchemaFromArgs(definition.args),
        };
      }
    }

    // Extract auth metadata if present
    let authMeta: { provider: string; methods: AuthMethod[] } | undefined;
    if (hooks.auth && typeof hooks.auth === "object") {
      authHook = hooks.auth as AuthHook;
      authMeta = {
        provider: authHook.provider,
        methods: authHook.methods.map((m) => ({
          type: m.type,
          label: m.label,
        })),
      };
    }

    sendResult(id, {
      name: params.pluginPath.split("/").pop()?.replace(/\.[tj]s$/, "") ?? "unknown",
      hooks: hookNames,
      tools: toolDefs,
      auth: authMeta,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `Failed to initialize plugin: ${msg}`);
  }
}

async function handleToolExecute(
  id: number,
  params: { tool: string; args: unknown; context: unknown },
): Promise<void> {
  const toolHook = (pluginHooks as { tool?: Record<string, ToolLike> }).tool;
  const def = toolHook?.[params.tool];
  if (!def || typeof def.execute !== "function") {
    sendError(id, -32601, `Tool not found: ${params.tool}`);
    return;
  }

  try {
    const result = await def.execute(params.args, params.context);
    if (typeof result === "string") {
      sendResult(id, { output: result });
      return;
    }
    const output = (result as { output?: unknown })?.output;
    sendResult(id, {
      output: typeof output === "string" ? output : JSON.stringify(output ?? result ?? null),
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `Tool ${params.tool} failed: ${msg}`);
  }
}

async function handleHookInvoke(
  id: number,
  params: { hook: string; input: unknown; output: unknown },
): Promise<void> {
  const handler = pluginHooks[params.hook];
  if (typeof handler !== "function") {
    sendError(id, -32601, `Hook not found: ${params.hook}`);
    return;
  }

  try {
    // TS parity: `config` hooks mutate the first argument in-place.
    // Use one shared object for both input/output so in-place edits are preserved.
    if (params.hook === "config") {
      const seed =
        (params.output as UnknownRecord | null) ??
        (params.input as UnknownRecord | null) ??
        ({} as UnknownRecord);
      const result = await handler(seed, seed);
      sendResult(id, { output: result ?? seed });
      return;
    }

    const result = await handler(params.input, params.output);
    sendResult(id, { output: result ?? params.output });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `Hook ${params.hook} failed: ${msg}`);
  }
}

async function handleAuthAuthorize(
  id: number,
  params: { methodIndex: number; inputs?: Record<string, string> },
): Promise<void> {
  if (!authHook?.authorize) {
    sendError(id, -32601, "No auth.authorize handler");
    return;
  }

  try {
    const method = authHook.methods[params.methodIndex];
    if (!method) {
      sendError(id, -32602, `Invalid method index: ${params.methodIndex}`);
      return;
    }

    const result = await authHook.authorize(method, params.inputs);
    // Stash callback for later auth.callback call
    if (result.callback) {
      pendingAuthCallback = result.callback;
    }

    sendResult(id, {
      url: result.url,
      instructions: result.instructions,
      method: result.method,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.authorize failed: ${msg}`);
  }
}

async function handleAuthCallback(
  id: number,
  params: { code?: string },
): Promise<void> {
  if (!pendingAuthCallback) {
    sendError(id, -32601, "No pending auth callback");
    return;
  }

  try {
    const result = await pendingAuthCallback(params.code);
    pendingAuthCallback = null;
    sendResult(id, result);
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.callback failed: ${msg}`);
  }
}

async function handleAuthLoad(id: number): Promise<void> {
  if (!authHook?.loader) {
    sendError(id, -32601, "No auth.loader handler");
    return;
  }

  try {
    const loaded = await authHook.loader();
    const hasCustomFetch = typeof loaded.fetch === "function";
    if (hasCustomFetch) {
      customFetch = loaded.fetch!;
    }

    sendResult(id, {
      apiKey: loaded.apiKey,
      hasCustomFetch,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.load failed: ${msg}`);
  }
}

async function handleAuthFetch(
  id: number,
  params: { url: string; method: string; headers: Record<string, string>; body?: string },
): Promise<void> {
  if (!customFetch) {
    sendError(id, -32601, "No custom fetch available");
    return;
  }

  try {
    const resp = await customFetch(params.url, {
      method: params.method,
      headers: params.headers,
      body: params.body,
    });

    const respHeaders: Record<string, string> = {};
    resp.headers.forEach((v: string, k: string) => {
      respHeaders[k] = v;
    });

    const body = await resp.text();
    sendResult(id, {
      status: resp.status,
      headers: respHeaders,
      body,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.fetch failed: ${msg}`);
  }
}

async function handleAuthFetchStream(
  id: number,
  params: { url: string; method: string; headers: Record<string, string>; body?: string },
): Promise<void> {
  if (!customFetch) {
    sendError(id, -32601, "No custom fetch available");
    return;
  }

  try {
    const resp = await customFetch(params.url, {
      method: params.method,
      headers: params.headers,
      body: params.body,
    });

    const respHeaders: Record<string, string> = {};
    resp.headers.forEach((v: string, k: string) => {
      respHeaders[k] = v;
    });

    // First response carries status/headers so Rust can begin the stream pipeline.
    sendResult(id, {
      status: resp.status,
      headers: respHeaders,
    });

    if (!resp.body) {
      sendNotification("auth.fetch.stream.end", { requestId: id });
      return;
    }

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value || value.length === 0) continue;
      const chunk = decoder.decode(value, { stream: true });
      if (chunk.length === 0) continue;
      sendNotification("auth.fetch.stream.chunk", {
        requestId: id,
        chunk,
      });
    }

    const rest = decoder.decode();
    if (rest.length > 0) {
      sendNotification("auth.fetch.stream.chunk", {
        requestId: id,
        chunk: rest,
      });
    }
    sendNotification("auth.fetch.stream.end", { requestId: id });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendNotification("auth.fetch.stream.error", {
      requestId: id,
      message: msg,
    });
    sendNotification("auth.fetch.stream.end", { requestId: id });
  }
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  // Set stdin to raw binary mode
  if (typeof process.stdin.setEncoding === "function") {
    // Don't call setEncoding — we want raw bytes
  }
  process.stdin.resume();

  // Independent of the read loop: if the parent dies while this host is busy inside a
  // plugin's initialize or a hook, the loop never regains control to observe EOF, and the
  // process would be orphaned mid-call. stdin 'end' fires regardless of what is pending.
  process.stdin.on("end", () => process.exit(0));
  process.stdin.on("close", () => process.exit(0));

  while (true) {
    let msg: JsonRpcRequest | null;
    try {
      msg = await readMessage();
    } catch {
      break; // stdin closed or parse error
    }

    if (msg === null) break; // EOF

    const { id, method, params } = msg;

    switch (method) {
      case "initialize":
        await handleInitialize(id, params as Parameters<typeof handleInitialize>[1]);
        break;
      case "hook.invoke":
        await handleHookInvoke(id, params as Parameters<typeof handleHookInvoke>[1]);
        break;
      case "tool.execute":
        await handleToolExecute(id, params as Parameters<typeof handleToolExecute>[1]);
        break;
      case "auth.authorize":
        await handleAuthAuthorize(id, params as Parameters<typeof handleAuthAuthorize>[1]);
        break;
      case "auth.callback":
        await handleAuthCallback(id, params as Parameters<typeof handleAuthCallback>[1]);
        break;
      case "auth.load":
        await handleAuthLoad(id);
        break;
      case "auth.fetch":
        await handleAuthFetch(id, params as Parameters<typeof handleAuthFetch>[1]);
        break;
      case "auth.fetch.stream":
        await handleAuthFetchStream(id, params as Parameters<typeof handleAuthFetchStream>[1]);
        break;
      case "shutdown":
        sendResult(id, {});
        process.exit(0);
      default:
        sendError(id, -32601, `Unknown method: ${method}`);
    }
  }

  // Reaching here means stdin hit EOF, i.e. the parent server is gone. Returning normally
  // is not enough: omo installs timers and a background manager that keep the event loop
  // alive, so the host would linger as an orphan holding its full resident memory.
  process.exit(0);
}

main().catch((err) => {
  process.stderr.write(`plugin-host fatal: ${err}\n`);
  process.exit(1);
});
