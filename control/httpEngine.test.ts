import { describe, expect, test } from "bun:test";
import { isPromptEcho, isPromptFinal, providerErrorText } from "./httpEngine.ts";

describe("run classification", () => {
  test("prompt.final proves the turn ended, not that it worked", () => {
    expect(isPromptFinal({ type: "session.updated", source: "prompt.final" })).toBe(true);
    expect(isPromptFinal({ type: "session.updated", properties: { source: "prompt.final" } })).toBe(true);
    expect(isPromptFinal({ type: "session.updated" })).toBe(false);
    expect(isPromptFinal({ type: "message.part.updated" })).toBe(false);
  });

  test("prompt echo is not model work, real answers are", () => {
    expect(
      isPromptEcho(
        "# RefundHunter — PROMPT.md\n> OMP HOME: /Users/vakandi/Documents/omp-server/subworkers/refund-hunter/ — all reads/writes stay inside this home",
      ),
    ).toBe(true);
    expect(isPromptEcho("# Elia – Personal Assistant for Wael Bousfira\n> OMP HOME: /x/")).toBe(true);
    expect(isPromptEcho("OK")).toBe(false);
    expect(isPromptEcho("I checked the workspace and found three refunded orders.")).toBe(false);
    expect(isPromptEcho("The PROMPT.md describes tools that need a category.")).toBe(false);
  });

  test("provider errors keep their bare reason", () => {
    expect(providerErrorText("Provider error: Network error: error sending request")).toBe(
      "Network error: error sending request",
    );
  });
});