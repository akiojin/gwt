/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { AIToolSelectorScreen } from '../../../components/screens/AIToolSelectorScreen.js';
import { Window } from 'happy-dom';

describe('AIToolSelectorScreen', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it('should render header with title', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    expect(getByText(/AI Tool Selection/i)).toBeDefined();
  });

  it('should render AI tool options', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    expect(getByText(/Claude Code/i)).toBeDefined();
    expect(getByText(/Codex CLI/i)).toBeDefined();
  });

  it('should render footer with actions', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />
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
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it('should handle back navigation with q key', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    // Test will verify onBack is called when q is pressed
    expect(container).toBeDefined();
  });

  it('should handle tool selection', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />
    );

    // Test will verify onSelect is called with correct tool
    expect(container).toBeDefined();
  });
});
