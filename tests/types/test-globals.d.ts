/// <reference types="bun-types/test-globals" />

import type { expect, Mock as BunMock } from "bun:test";
import type { TestingLibraryMatchers } from "@testing-library/jest-dom/types/matchers";

declare module "bun:test" {
  interface Matchers<T = any> extends TestingLibraryMatchers<
    ReturnType<typeof expect.stringContaining>,
    T
  > {}

  interface Matchers<T = any> {
    toBeInTheDocument(): void;
    toBeEnabled(): void;
    toBeDisabled(): void;
  }
}

declare global {
  type Mock<T extends (...args: any[]) => any = (...args: any[]) => any> =
    BunMock<T>;
  type MockedFunction<
    T extends (...args: any[]) => any = (...args: any[]) => any,
  > = BunMock<T>;
}

export {};
