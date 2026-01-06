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
      {/* Background overlay - fills entire screen to hide background */}
      <box
        position="absolute"
        top={0}
        left={0}
        width="100%"
        height="100%"
        zIndex={50}
        backgroundColor="black"
      />
      {/* Popup content */}
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
