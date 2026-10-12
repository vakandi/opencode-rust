/**
 * echo-plugin.ts — minimal test plugin that echoes hook invocations.
 *
 * Usage: configure as `file:///path/to/echo-plugin.ts` in opencode.json.
 */

export default async function echoPlugin(_input: unknown) {
  return {
    "chat.headers": async (
      _input: unknown,
      output: Record<string, unknown>,
    ) => {
      const headers = (output.headers ?? {}) as Record<string, string>;
      headers["X-Echo-Plugin"] = "active";
      return { ...output, headers };
    },

    "tool.execute.before": async (
      input: unknown,
      output: unknown,
    ) => {
      // Pass through unchanged
      return output;
    },
  };
}
