import { describe, expect, it } from "vitest";
import { maskSensitiveData } from "./privacyMask";

describe("maskSensitiveData", () => {
  it("masks Anthropic API keys", () => {
    const input = "Error with key sk-ant-api03-abc123XYZ_def456";
    expect(maskSensitiveData(input)).toBe("Error with key [REDACTED:API_KEY]");
  });

  it("masks OpenAI API keys", () => {
    const input = "Using sk-proj-abcdefghijklmnopqrstuvwx";
    expect(maskSensitiveData(input)).toBe("Using [REDACTED:API_KEY]");
  });

  it("masks GitHub personal access tokens", () => {
    const input = "Token: ghp_1234567890abcdefghijklmnopqrstuvwxyz";
    expect(maskSensitiveData(input)).toBe("Token: [REDACTED:GITHUB_TOKEN]");
  });

  it("masks GitHub OAuth tokens", () => {
    const input = "OAuth: gho_1234567890abcdefghijklmnopqrstuvwxyz";
    expect(maskSensitiveData(input)).toBe("OAuth: [REDACTED:GITHUB_TOKEN]");
  });

  it("masks GitHub fine-grained PATs", () => {
    const input = "PAT: github_pat_11ABC123DEF456GHI789";
    expect(maskSensitiveData(input)).toBe("PAT: [REDACTED:GITHUB_PAT]");
  });

  it("masks Bearer tokens", () => {
    const input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc";
    expect(maskSensitiveData(input)).toBe("Authorization: Bearer [REDACTED]");
  });

  it("masks password fields", () => {
    expect(maskSensitiveData("password: mysecret123")).toBe("password: [REDACTED]");
    expect(maskSensitiveData("password=mysecret123")).toBe("password=[REDACTED]");
    expect(maskSensitiveData("Password: MyPass!")).toBe("Password: [REDACTED]");
  });

  it("masks environment variables with sensitive names", () => {
    expect(maskSensitiveData("ANTHROPIC_API_KEY=sk-ant-something")).toBe(
      "ANTHROPIC_API_KEY=[REDACTED]",
    );
    expect(maskSensitiveData("AUTH_TOKEN: abc123def")).toBe("AUTH_TOKEN: [REDACTED]");
    expect(maskSensitiveData("MY_SECRET=supersecret")).toBe("MY_SECRET=[REDACTED]");
  });

  it("does not mask normal text", () => {
    const input = "This is a normal error message";
    expect(maskSensitiveData(input)).toBe(input);
  });

  it("masks only sensitive parts in mixed content", () => {
    const input =
      "Failed to authenticate with key sk-ant-api03-xyz123 at endpoint https://api.example.com";
    const result = maskSensitiveData(input);
    expect(result).toContain("[REDACTED:API_KEY]");
    expect(result).toContain("https://api.example.com");
    expect(result).not.toContain("sk-ant-api03-xyz123");
  });
});
