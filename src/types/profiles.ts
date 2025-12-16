/**
 * 環境変数プロファイル型定義
 *
 * プロファイル機能で使用する型を定義します。
 * @see specs/SPEC-dafff079/spec.md
 */

/**
 * 環境変数プロファイル
 *
 * 環境変数のセットを表します。
 */
export interface EnvironmentProfile {
  /** プロファイルの表示名 */
  displayName: string;
  /** プロファイルの説明（オプション） */
  description?: string;
  /** 環境変数のキーバリューペア */
  env: Record<string, string>;
}

/**
 * プロファイル設定
 *
 * 全プロファイルを管理する設定です。
 * ~/.gwt/profiles.yaml に保存されます。
 */
export interface ProfilesConfig {
  /** 設定ファイルのバージョン */
  version: string;
  /** 現在アクティブなプロファイル名（nullの場合はプロファイルなし） */
  activeProfile: string | null;
  /** プロファイルの辞書（キー: プロファイル名、値: プロファイル設定） */
  profiles: Record<string, EnvironmentProfile>;
}

/**
 * デフォルトのプロファイル設定
 *
 * profiles.yamlが存在しない場合に使用されます。
 * 不変オブジェクトとして扱われるため、変更は禁止されています。
 */
export const DEFAULT_PROFILES_CONFIG: Readonly<ProfilesConfig> = Object.freeze({
  version: "1.0",
  activeProfile: null,
  profiles: Object.freeze({}),
});

/**
 * プロファイル名のバリデーションパターン
 *
 * 小文字英数字とハイフンのみを許可し、先頭と末尾は英数字でなければなりません。
 */
export const PROFILE_NAME_PATTERN = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/;

/**
 * プロファイル名をバリデート
 *
 * @param name - 検証するプロファイル名
 * @returns バリデーション結果
 */
export function isValidProfileName(name: string): boolean {
  return name.length > 0 && PROFILE_NAME_PATTERN.test(name);
}
