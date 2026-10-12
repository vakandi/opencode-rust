/**
 * Minimal bundled auth plugin for GitHub Copilot.
 *
 * Supports standard and enterprise login methods through the same callback flow.
 * The callback expects a pasted token/code and stores it as OAuth access/refresh.
 */

function normalizeDomain(value: string): string {
  return value.replace(/^https?:\/\//, "").replace(/\/$/, "");
}

export default async function CopilotAuthPlugin() {
  return {
    auth: {
      provider: "github-copilot",
      methods: [
        {
          type: "oauth",
          label: "GitHub Copilot (paste token)",
        },
        {
          type: "oauth",
          label: "GitHub Copilot Enterprise (paste token)",
        },
      ],
      async authorize(method: { label?: string }, inputs?: Record<string, string>) {
        const enterprise = (method?.label ?? "").toLowerCase().includes("enterprise");
        const enterpriseUrl = inputs?.enterpriseUrl ? normalizeDomain(inputs.enterpriseUrl) : undefined;
        return {
          method: "code",
          instructions: enterprise
            ? "Paste your GitHub Enterprise Copilot token/code."
            : "Paste your GitHub Copilot token/code.",
          callback: async (code?: string) => {
            const value = code?.trim();
            if (!value) return { type: "failed" };
            if (enterprise) {
              return {
                type: "success",
                provider: "github-copilot-enterprise",
                enterpriseUrl: enterpriseUrl,
                access: value,
                refresh: value,
                expires: 0,
              };
            }
            return {
              type: "success",
              access: value,
              refresh: value,
              expires: 0,
            };
          },
        };
      },
      async loader() {
        const key =
          process.env.GITHUB_COPILOT_API_KEY?.trim() ||
          process.env.GITHUB_TOKEN?.trim();
        if (!key) return {};
        return {
          apiKey: key,
        };
      },
    },
  };
}
