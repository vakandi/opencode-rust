/**
 * Minimal bundled auth plugin for OpenAI/Codex.
 *
 * This implementation intentionally keeps behavior simple for Rust plugin-host
 * compatibility: auth.authorize returns a callback that expects an auth code
 * (or pasted token) and stores it as OAuth access/refresh values.
 */

export default async function CodexAuthPlugin() {
  return {
    auth: {
      provider: "openai",
      methods: [
        {
          type: "oauth",
          label: "ChatGPT/Codex OAuth (paste code)",
        },
      ],
      async authorize() {
        return {
          method: "code",
          instructions:
            "Open ChatGPT/Codex authorization and paste the returned code/token here.",
          callback: async (code?: string) => {
            const value = code?.trim();
            if (!value) return { type: "failed" };
            return {
              type: "success",
              access: value,
              refresh: value,
              expires: Date.now() + 3600 * 1000,
            };
          },
        };
      },
      async loader() {
        const key = process.env.OPENAI_API_KEY?.trim();
        if (!key) return {};
        return {
          apiKey: key,
        };
      },
    },
    "chat.headers": async () => {
      return {
        originator: "opencode",
        "x-title": "opencode",
      };
    },
  };
}
