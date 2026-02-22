/**
 * Mask sensitive data in text before sending to external services.
 * Patterns: API keys, tokens, passwords, secrets.
 */

const MASK_PATTERNS: [RegExp, string][] = [
  // Anthropic API keys
  [/sk-ant-[A-Za-z0-9_-]+/g, "[REDACTED:API_KEY]"],
  // Generic sk- keys (OpenAI etc.)
  [/sk-[A-Za-z0-9_-]{20,}/g, "[REDACTED:API_KEY]"],
  // GitHub personal access tokens
  [/ghp_[A-Za-z0-9]{36,}/g, "[REDACTED:GITHUB_TOKEN]"],
  // GitHub OAuth tokens
  [/gho_[A-Za-z0-9]{36,}/g, "[REDACTED:GITHUB_TOKEN]"],
  // GitHub fine-grained PATs
  [/github_pat_[A-Za-z0-9_]{20,}/g, "[REDACTED:GITHUB_PAT]"],
  // Bearer tokens
  [/Bearer\s+[A-Za-z0-9_.\-]+/g, "Bearer [REDACTED]"],
  // password= or password: followed by non-space value
  [/(password\s*[:=]\s*)\S+/gi, "$1[REDACTED]"],
  // Environment variable patterns with KEY/TOKEN/SECRET/PASSWORD in name
  [/([A-Za-z_]*(?:KEY|TOKEN|SECRET|PASSWORD)[A-Za-z_]*\s*[:=]\s*)\S+/g, "$1[REDACTED]"],
];

export function maskSensitiveData(text: string): string {
  let result = text;
  for (const [pattern, replacement] of MASK_PATTERNS) {
    result = result.replace(pattern, replacement);
  }
  return result;
}
