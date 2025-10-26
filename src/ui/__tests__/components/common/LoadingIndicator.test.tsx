/**
 * @vitest-environment happy-dom
 */
import React from 'react';
import { describe, it, expect, beforeEach } from 'vitest';
import { act, render } from '@testing-library/react';
import { LoadingIndicator } from '../../../components/common/LoadingIndicator.js';
import { Window } from 'happy-dom';

beforeEach(() => {
  const window = new Window();
  globalThis.window = window as any;
  globalThis.document = window.document as any;
});

describe('LoadingIndicator', () => {
  const getSpinnerText = (container: HTMLElement) => {
    return container.querySelector('ink-text')?.textContent ?? '';
  };

  const getMessageText = (container: HTMLElement) => {
    const texts = container.querySelectorAll('ink-text');
    return texts.length > 1 ? texts[1]?.textContent ?? '' : '';
  };

  it('does not render before the delay elapses', async () => {
    const { container } = render(
      <LoadingIndicator isLoading={true} message="読み込み中" delay={50} />
    );

    expect(container.textContent).toBe('');

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });

    expect(container.textContent).toBe('');
  });

  it('renders after the delay elapses', async () => {
    const { container } = render(
      <LoadingIndicator isLoading={true} message="読み込み中" delay={30} />
    );

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 40));
    });

    expect(getMessageText(container)).toContain('読み込み中');
  });

  it('stops rendering when loading becomes false', async () => {
    const { container, rerender } = render(
      <LoadingIndicator isLoading={true} message="読み込み中" delay={10} />
    );

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 15));
    });

    expect(getMessageText(container)).toContain('読み込み中');

    await act(async () => {
      rerender(<LoadingIndicator isLoading={false} message="読み込み中" delay={10} />);
      await new Promise((resolve) => setTimeout(resolve, 5));
    });

    expect(container.textContent).toBe('');
  });

  it('cycles through spinner frames over time', async () => {
    const customFrames = ['.', '..', '...'];
    const { container } = render(
      <LoadingIndicator
        isLoading={true}
        message="読み込み中"
        delay={0}
        interval={5}
        frames={customFrames}
      />
    );

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 6));
    });

    const firstFrame = getSpinnerText(container);

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 6));
    });

    const secondFrame = getSpinnerText(container);

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 6));
    });

    const thirdFrame = getSpinnerText(container);

    expect(secondFrame).not.toEqual(firstFrame);
    expect(thirdFrame).not.toEqual(secondFrame);
    expect(customFrames).toContain(firstFrame ?? '');
    expect(customFrames).toContain(secondFrame ?? '');
    expect(customFrames).toContain(thirdFrame ?? '');
    expect(getMessageText(container)).toContain('読み込み中');
  });
});
