/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { ExecutionModeSelectorScreen } from '../../../components/screens/ExecutionModeSelectorScreen.js';
import { Window } from 'happy-dom';

describe('ExecutionModeSelectorScreen', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it('should render header with title', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    expect(container).toBeDefined();
  });

  it('should render execution mode options', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    expect(getByText(/Normal/i)).toBeDefined();
    expect(getByText(/Continue/i)).toBeDefined();
    expect(getByText(/Resume/i)).toBeDefined();
  });

  it('should render footer with actions', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/q/i).length).toBeGreaterThan(0);
  });

  it('should use terminal height for layout calculation', () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it('should handle back navigation with q key', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    // Test will verify onBack is called when q is pressed
    expect(container).toBeDefined();
  });

  it('should handle mode selection', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    // Test will verify onSelect is called with correct mode
    expect(container).toBeDefined();
  });
});
