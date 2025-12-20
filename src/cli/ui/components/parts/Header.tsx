import React from "react";
import { Box, Text } from "ink";

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

/**
 * Header component - displays title and optional divider
 * Optimized with React.memo to prevent unnecessary re-renders
 */
export const Header = React.memo(function Header({
  title,
  titleColor = "cyan",
  dividerChar = "─",
  showDivider = true,
  width = 80,
  version,
  workingDirectory,
  activeProfile,
}: HeaderProps) {
  const divider = dividerChar.repeat(width);

  // タイトル構築: "gwt v2.14 | Profile: development" または "gwt v2.14 | Profile: (none)"
  let displayTitle = version ? `${title} v${version}` : title;
  if (activeProfile !== undefined) {
    const profileLabel = activeProfile ?? "(none)";
    displayTitle = `${displayTitle} | Profile: ${profileLabel}`;
  }

  return (
    <Box flexDirection="column">
      <Box>
        <Text bold color={titleColor}>
          {displayTitle}
        </Text>
      </Box>
      {showDivider && (
        <Box>
          <Text dimColor>{divider}</Text>
        </Box>
      )}
      {workingDirectory && (
        <Box>
          <Text dimColor>Working Directory: </Text>
          <Text>{workingDirectory}</Text>
        </Box>
      )}
    </Box>
  );
});
