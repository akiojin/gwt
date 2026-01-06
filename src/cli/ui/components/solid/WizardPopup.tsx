/** @jsxImportSource @opentui/solid */
import { Show } from "solid-js";
import type { JSX } from "solid-js";

export interface WizardPopupProps {
  visible: boolean;
  onClose: () => void;
  onComplete: (result: unknown) => void;
  children?: JSX.Element;
}

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
      >
        {props.children}
      </box>
    </Show>
  );
}
