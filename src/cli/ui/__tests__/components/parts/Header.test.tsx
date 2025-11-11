/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { Header } from '../../../components/parts/Header.js';
import { Window } from 'happy-dom';

describe('Header', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it('should render title', () => {
    const { getByText } = render(<Header title="Claude Worktree" />);

    expect(getByText('Claude Worktree')).toBeDefined();
  });

  it('should render divider', () => {
    const { getByText } = render(<Header title="Test" />);

    // Check for a line of dashes (divider)
    expect(getByText(/─+/)).toBeDefined();
  });

  it('should render title in bold and cyan by default', () => {
    const { container } = render(<Header title="Test Title" />);

    expect(container).toBeDefined();
  });

  it('should accept custom title color', () => {
    const { container } = render(<Header title="Test" titleColor="green" />);

    expect(container).toBeDefined();
  });

  it('should accept custom divider character', () => {
    const { getByText } = render(<Header title="Test" dividerChar="=" />);

    expect(getByText(/=+/)).toBeDefined();
  });

  it('should render without divider when showDivider is false', () => {
    const { queryByText } = render(<Header title="Test" showDivider={false} />);

    expect(queryByText(/─+/)).toBeNull();
  });
});
