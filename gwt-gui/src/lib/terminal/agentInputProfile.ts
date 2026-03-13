/**
 * Agent input profiles define how semantic actions (Send, Queue, Interrupt)
 * are translated to PTY byte sequences for each agent CLI.
 */

export interface AgentInputProfile {
  agentId: string;
  /** Text send action — suffix bytes appended after text */
  send: { suffixBytes: number[] };
  /** Queue action (e.g. Codex Tab) — optional */
  queue?: { suffixBytes: number[] };
  /** Interrupt action — bytes sent to PTY */
  interrupt: { bytes: number[] };
  /** Clear input action — optional */
  clear?: { bytes: number[] };
  /** Image input support */
  imageSupport?: {
    method: "path_reference" | "command";
    commandTemplate?: string;
  };
  /** Bytes used for newlines within multiline text */
  newlineBytes: number[];
}

const profiles: Record<string, AgentInputProfile> = {
  claude: {
    agentId: "claude",
    send: { suffixBytes: [0x0d] },
    interrupt: { bytes: [0x1b] },
    imageSupport: { method: "path_reference" },
    newlineBytes: [0x0a],
  },
  codex: {
    agentId: "codex",
    send: { suffixBytes: [0x0d] },
    queue: { suffixBytes: [0x09] },
    interrupt: { bytes: [0x1b] },
    clear: { bytes: [0x03] },
    imageSupport: { method: "path_reference" },
    newlineBytes: [0x0a],
  },
  gemini: {
    agentId: "gemini",
    send: { suffixBytes: [0x0d] },
    interrupt: { bytes: [0x1b] },
    imageSupport: { method: "command", commandTemplate: "@{path}" },
    newlineBytes: [0x5c, 0x0d],
  },
  copilot: {
    agentId: "copilot",
    send: { suffixBytes: [0x0d] },
    interrupt: { bytes: [0x1b] },
    clear: { bytes: [0x03] },
    imageSupport: { method: "command", commandTemplate: "@{path}" },
    newlineBytes: [0x0a],
  },
  opencode: {
    agentId: "opencode",
    send: { suffixBytes: [0x0d] },
    interrupt: { bytes: [0x1b] },
    newlineBytes: [0x0a],
  },
};

/** Default profile used when the agent is not recognized. Sends Enter (CR). */
const defaultProfile: AgentInputProfile = {
  agentId: "_default",
  send: { suffixBytes: [0x0d] },
  interrupt: { bytes: [0x1b] },
  newlineBytes: [0x0a],
};

/** Lookup agent profile by agentId. Returns undefined for unknown agents. */
export function getAgentInputProfile(
  agentId: string,
): AgentInputProfile | undefined {
  return profiles[agentId];
}

/** Lookup agent profile with fallback to a sensible default. */
export function getAgentInputProfileOrDefault(
  agentId: string,
): AgentInputProfile {
  return profiles[agentId] ?? defaultProfile;
}

/** Encode text + send suffix as byte array for PTY write. */
export function buildSendBytes(
  profile: AgentInputProfile,
  text: string,
): number[] {
  const bytes = encodeTextWithNewlines(text, profile.newlineBytes);
  return [...bytes, ...profile.send.suffixBytes];
}

/** Encode text + queue suffix. Returns null if profile has no queue support. */
export function buildQueueBytes(
  profile: AgentInputProfile,
  text: string,
): number[] | null {
  if (!profile.queue) return null;
  const bytes = encodeTextWithNewlines(text, profile.newlineBytes);
  return [...bytes, ...profile.queue.suffixBytes];
}

/** Get interrupt byte sequence. */
export function buildInterruptBytes(profile: AgentInputProfile): number[] {
  return [...profile.interrupt.bytes];
}

/** Build image reference string. Returns null if unsupported. */
export function buildImageReference(
  profile: AgentInputProfile,
  imagePath: string,
): string | null {
  if (!profile.imageSupport) return null;
  if (profile.imageSupport.method === "path_reference") {
    return imagePath;
  }
  if (profile.imageSupport.commandTemplate) {
    return profile.imageSupport.commandTemplate.replace("{path}", imagePath);
  }
  return imagePath;
}

/** Encode text, replacing \n with profile-specific newline bytes. */
function encodeTextWithNewlines(
  text: string,
  newlineBytes: number[],
): number[] {
  const encoder = new TextEncoder();
  const lines = text.split("\n");
  const result: number[] = [];
  for (let i = 0; i < lines.length; i++) {
    if (i > 0) {
      result.push(...newlineBytes);
    }
    result.push(...encoder.encode(lines[i]));
  }
  return result;
}
