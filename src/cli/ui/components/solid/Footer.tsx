/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";

export interface FooterAction {
  key: string;
  description: string;
}

export interface FooterProps {
  actions: FooterAction[];
  separator?: string;
}

export function Footer({ actions, separator = "  " }: FooterProps) {
  if (actions.length === 0) {
    return null;
  }

  return (
    <box flexDirection="row">
      {actions.map((action, index) => (
        <>
          <text attributes={TextAttributes.DIM}>[</text>
          <text fg="cyan" attributes={TextAttributes.BOLD}>
            {action.key}
          </text>
          <text attributes={TextAttributes.DIM}>]</text>
          <text>{` ${action.description}`}</text>
          {index < actions.length - 1 ? (
            <text attributes={TextAttributes.DIM}>{separator}</text>
          ) : null}
        </>
      ))}
    </box>
  );
}
