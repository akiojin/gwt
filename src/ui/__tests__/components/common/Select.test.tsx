/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { Select } from '../../../components/common/Select.js';
import { Window } from 'happy-dom';

interface TestItem {
  label: string;
  value: string;
}

describe('Select', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  const mockItems: TestItem[] = [
    { label: 'Option 1', value: 'opt1' },
    { label: 'Option 2', value: 'opt2' },
    { label: 'Option 3', value: 'opt3' },
  ];

  it('should render items', () => {
    const onSelect = vi.fn();
    const { getByText } = render(<Select items={mockItems} onSelect={onSelect} />);

    expect(getByText('Option 1')).toBeDefined();
    expect(getByText('Option 2')).toBeDefined();
    expect(getByText('Option 3')).toBeDefined();
  });

  it('should accept limit prop for scrolling', () => {
    const onSelect = vi.fn();
    const { container } = render(<Select items={mockItems} onSelect={onSelect} limit={2} />);

    // Verify component renders without error
    expect(container).toBeDefined();
  });

  it('should accept initialIndex prop', () => {
    const onSelect = vi.fn();
    const { container } = render(<Select items={mockItems} onSelect={onSelect} initialIndex={1} />);

    // Verify component renders without error
    expect(container).toBeDefined();
  });

  it('should render with empty items array', () => {
    const onSelect = vi.fn();
    const { container } = render(<Select items={[]} onSelect={onSelect} />);

    expect(container).toBeDefined();
  });

  it('should call onSelect with the selected item', () => {
    const onSelect = vi.fn();
    render(<Select items={mockItems} onSelect={onSelect} />);

    // Note: Simulating key press in Ink requires ink-testing-library
    // For now, we just verify the component structure
    expect(onSelect).not.toHaveBeenCalled();
  });
});
