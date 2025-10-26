import '@testing-library/jest-dom';
import { cleanup } from '@testing-library/react';
import { afterEach, beforeAll, afterAll } from 'vitest';

// Cleanup after each test
afterEach(() => {
  cleanup();
});

// Suppress React act() warnings in tests
// These warnings are expected when testing Ink components
// which use internal state updates that can't be wrapped in act()
const originalError = console.error;
beforeAll(() => {
  console.error = (...args: any[]) => {
    if (
      typeof args[0] === 'string' &&
      args[0].includes('An update to') &&
      args[0].includes('inside a test was not wrapped in act')
    ) {
      return;
    }
    originalError.call(console, ...args);
  };
});

afterAll(() => {
  console.error = originalError;
});
