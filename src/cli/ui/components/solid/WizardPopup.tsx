/** @jsxImportSource @opentui/solid */
import { Show, createSignal, createEffect } from "solid-js";
import { useKeyboard } from "@opentui/solid";

export interface WizardPopupProps {
  visible: boolean;
  onClose: () => void;
  onComplete: (result: unknown) => void;
  children?: unknown;
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
 */
export function WizardPopup(props: WizardPopupProps) {
  const [step, setStep] = createSignal(0);

  // Reset step when popup becomes visible
  createEffect(() => {
    if (props.visible) {
      setStep(0);
    }
  });

  // Handle keyboard events
  useKeyboard((key) => {
    if (!props.visible) {
      return;
    }

    if (key.name === "escape") {
      if (step() > 0) {
        // Go back to previous step
        setStep((s) => s - 1);
      } else {
        // Close wizard on first step
        props.onClose();
      }
    }
  });

  return (
    <Show when={props.visible}>
      {/* Background overlay */}
      <box
        position="absolute"
        top={0}
        left={0}
        width="100%"
        height="100%"
        zIndex={50}
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
        flexDirection="column"
        padding={1}
      >
        <text bold color="cyan">
          Select
        </text>
        <text> </text>
        <text>Step {step() + 1}</text>
        {props.children}
      </box>
    </Show>
  );
}
