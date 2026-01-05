import { TextAttributes } from "@opentui/core";

export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
  /**
   * アプリケーションのバージョン文字列
   * - string: バージョンが利用可能（例: "1.12.3"）
   * - null: バージョン取得失敗
   * - undefined: バージョン未提供（後方互換性のため）
   * @default undefined
   */
  version?: string | null | undefined;
  /**
   * 起動時の作業ディレクトリの絶対パス
   * - string: ディレクトリパスが利用可能
   * - undefined: ディレクトリ情報未提供
   * @default undefined
   */
  workingDirectory?: string;
  /**
   * 現在アクティブなプロファイル名
   * - string: プロファイル名が利用可能（例: "development"）
   * - null: プロファイルが選択されていない
   * - undefined: プロファイル情報未提供
   * @default undefined
   */
  activeProfile?: string | null | undefined;
}

export function Header({
  title,
  titleColor = "cyan",
  dividerChar = "─",
  showDivider = true,
  width = 80,
  version,
  workingDirectory,
  activeProfile,
}: HeaderProps) {
  const divider = dividerChar.repeat(Math.max(0, width));

  let displayTitle = version ? `${title} v${version}` : title;
  if (activeProfile !== undefined) {
    const profileLabel = activeProfile ?? "(none)";
    displayTitle = `${displayTitle} | Profile: ${profileLabel}`;
  }

  return (
    <box flexDirection="column">
      <text fg={titleColor} attributes={TextAttributes.BOLD}>
        {displayTitle}
      </text>
      {showDivider && <text attributes={TextAttributes.DIM}>{divider}</text>}
      {workingDirectory && (
        <text>
          <span attributes={TextAttributes.DIM}>Working Directory: </span>
          <span>{workingDirectory}</span>
        </text>
      )}
    </box>
  );
}
