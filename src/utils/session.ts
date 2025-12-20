/**
 * Session utilities - backward compatibility layer
 *
 * This file re-exports all session-related functions and types from the
 * modularized session directory structure. Existing imports from this file
 * will continue to work without modification.
 *
 * For new code, consider importing directly from:
 * - ./session/types.js - Type definitions
 * - ./session/common.js - Common utilities
 * - ./session/parsers/claude.js - Claude-specific functions
 * - ./session/parsers/codex.js - Codex-specific functions
 * - ./session/parsers/gemini.js - Gemini-specific functions
 */

export * from "./session/index.js";
