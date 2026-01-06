/** @jsxImportSource @opentui/solid */
import { Show } from "solid-js";

export interface WizardPopupProps {
  visible: boolean;
  onClose: () => void;
  onComplete: (result: unknown) => void;
}

/**
 * WizardPopup - ブランチ選択後のウィザードポップアップ
 *
 * FR-044: ブランチ選択時にレイヤー表示
 * FR-045: 背景を半透過オーバーレイで覆う
 * FR-046: z-indexで前面表示
 * FR-047: ステップを同一ポップアップ内で切り替え
 * FR-048: キーバインドヘルプ表示
 * FR-049: Escapeでウィザード終了
 *
 * TODO: 実装予定（TDD RED状態）
 */
export function WizardPopup(_props: WizardPopupProps) {
  return (
    <Show when={_props.visible}>
      <box
        position="absolute"
        top={0}
        left={0}
        width="100%"
        height="100%"
        zIndex={50}
      >
        {/* TODO: 背景オーバーレイ */}
      </box>
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
      >
        <text>Select</text>
        {/* TODO: ステップコンテンツ */}
      </box>
    </Show>
  );
}
