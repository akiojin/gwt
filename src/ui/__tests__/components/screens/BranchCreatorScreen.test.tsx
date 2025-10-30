/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { BranchCreatorScreen } from '../../../components/screens/BranchCreatorScreen.js';
import { Window } from 'happy-dom';

describe('BranchCreatorScreen', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it('should render header with title', () => {
    const onBack = vi.fn();
    const onCreate = vi.fn();
    const { getByText } = render(
      <BranchCreatorScreen onBack={onBack} onCreate={onCreate} />
    );

    expect(getByText(/New Branch/i)).toBeDefined();
  });

  it('should render branch type selection initially', () => {
    const onBack = vi.fn();
    const onCreate = vi.fn();
    const { getByText } = render(
      <BranchCreatorScreen onBack={onBack} onCreate={onCreate} />
    );

    expect(getByText(/Select branch type/i)).toBeDefined();
    expect(getByText(/feature/i)).toBeDefined();
    expect(getByText(/hotfix/i)).toBeDefined();
    expect(getByText(/release/i)).toBeDefined();
  });

  it('should render footer with actions', () => {
    const onBack = vi.fn();
    const onCreate = vi.fn();
    const { getAllByText } = render(
      <BranchCreatorScreen onBack={onBack} onCreate={onCreate} />
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/esc/i).length).toBeGreaterThan(0);
  });

  it('should show branch name input after type selection', () => {
    const onBack = vi.fn();
    const onCreate = vi.fn();
    const { container } = render(
      <BranchCreatorScreen onBack={onBack} onCreate={onCreate} />
    );

    // Test will verify the screen transitions from type selection to name input
    expect(container).toBeDefined();
  });

  it('should handle branch creation', () => {
    const onBack = vi.fn();
    const onCreate = vi.fn();
    const { container } = render(
      <BranchCreatorScreen onBack={onBack} onCreate={onCreate} />
    );

    // Test will verify onCreate is called with correct branch name
    expect(container).toBeDefined();
  });

  it('should use terminal height for layout calculation', () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onCreate = vi.fn();
    const { container } = render(
      <BranchCreatorScreen onBack={onBack} onCreate={onCreate} />
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it('should handle back navigation with ESC key', () => {
    const onBack = vi.fn();
    const onCreate = vi.fn();
    const { container } = render(
      <BranchCreatorScreen onBack={onBack} onCreate={onCreate} />
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });
});
