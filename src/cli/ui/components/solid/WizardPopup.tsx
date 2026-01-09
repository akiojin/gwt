/** @jsxImportSource @opentui/solid */
import type { ScrollBoxRenderable } from "@opentui/core";
import { createContext, Show, useContext } from "solid-js";
import type { JSX } from "solid-js";

export interface WizardPopupProps {
  visible: boolean;
  onClose: () => void;
  onComplete: (result: unknown) => void;
  children?: JSX.Element;
}

interface WizardScrollContextValue {
  scrollByLines: (delta: number) => boolean;
  ensureLineVisible: (lineIndex: number) => boolean;
  canScrollUp: () => boolean;
  canScrollDown: () => boolean;
}

const WizardScrollContext = createContext<WizardScrollContextValue | null>(
  null,
);

export const useWizardScroll = () => useContext(WizardScrollContext);

/**
 * WizardPopup - ブランチ選択後のウィザードポップアップコンテナ
 *
 * FR-044: ブランチ選択時にレイヤー表示
 * FR-045: 背景を不透明オーバーレイで覆う
 * FR-046: z-indexで前面表示
 *
 * ステップ管理とキーボード処理はWizardControllerで行う
 */
export function WizardPopup(props: WizardPopupProps) {
  let scrollRef: ScrollBoxRenderable | undefined;

  const getScrollState = () => {
    if (!scrollRef) {
      return { scrollTop: 0, maxScrollTop: 0 };
    }
    const maxScrollTop = Math.max(
      0,
      scrollRef.scrollHeight - scrollRef.viewport.height,
    );
    return { scrollTop: scrollRef.scrollTop, maxScrollTop };
  };

  const scrollByLines = (delta: number) => {
    if (!scrollRef || delta === 0) {
      return false;
    }
    const { scrollTop, maxScrollTop } = getScrollState();
    if (delta < 0 && scrollTop <= 0) {
      return false;
    }
    if (delta > 0 && scrollTop >= maxScrollTop) {
      return false;
    }
    scrollRef.scrollBy(delta, "absolute");
    return true;
  };

  const ensureLineVisible = (lineIndex: number) => {
    if (!scrollRef) {
      return false;
    }
    const viewportHeight = scrollRef.viewport.height;
    if (viewportHeight <= 0) {
      return false;
    }
    const safeIndex = Math.max(0, lineIndex);
    const { scrollTop, maxScrollTop } = getScrollState();
    if (safeIndex < scrollTop) {
      scrollRef.scrollTo(Math.max(0, safeIndex));
      return true;
    }
    if (safeIndex >= scrollTop + viewportHeight) {
      const nextTop = Math.min(
        maxScrollTop,
        Math.max(0, safeIndex - viewportHeight + 1),
      );
      scrollRef.scrollTo(nextTop);
      return true;
    }
    return false;
  };

  const scrollContextValue: WizardScrollContextValue = {
    scrollByLines,
    ensureLineVisible,
    canScrollUp: () => getScrollState().scrollTop > 0,
    canScrollDown: () =>
      getScrollState().scrollTop < getScrollState().maxScrollTop,
  };

  return (
    <Show when={props.visible}>
      {/* Popup content - centered overlay without full-screen background */}
      <box
        position="absolute"
        top="20%"
        left="20%"
        width="60%"
        height="60%"
        zIndex={100}
        border
        borderStyle="single"
        borderColor="cyan"
        backgroundColor="black"
        flexDirection="column"
        padding={1}
        overflow="hidden"
      >
        <WizardScrollContext.Provider value={scrollContextValue}>
          <scrollbox
            ref={(node) => {
              scrollRef = node;
            }}
            flexGrow={1}
            flexShrink={1}
            minHeight={0}
            scrollX={false}
            scrollY
            scrollbarOptions={{ visible: false }}
            verticalScrollbarOptions={{ visible: false }}
            horizontalScrollbarOptions={{ visible: false }}
          >
            {props.children}
          </scrollbox>
        </WizardScrollContext.Provider>
      </box>
    </Show>
  );
}
