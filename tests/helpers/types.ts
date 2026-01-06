import type { mock, spyOn } from "bun:test";

/**
 * Mock type for bun:test - represents a mocked function
 */
export type Mock<T extends (...args: unknown[]) => unknown = (...args: unknown[]) => unknown> =
  ReturnType<typeof mock<T>>;

/**
 * SpyOn type for bun:test - represents a spied function
 */
export type SpyInstance = ReturnType<typeof spyOn>;
