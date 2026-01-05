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
    <text>
      {actions.map((action, index) => (
        <span>
          <span attributes={TextAttributes.DIM}>[</span>
          <span fg="cyan" attributes={TextAttributes.BOLD}>
            {action.key}
          </span>
          <span attributes={TextAttributes.DIM}>]</span>
          <span>{` ${action.description}`}</span>
          {index < actions.length - 1 && (
            <span attributes={TextAttributes.DIM}>{separator}</span>
          )}
        </span>
      ))}
    </text>
  );
}
