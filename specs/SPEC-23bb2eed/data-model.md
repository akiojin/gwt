# データモデル: semantic-release 設定構造

**仕様ID**: `SPEC-23bb2eed` | **日付**: 2025-10-25 | **実装計画**: [plan.md](./plan.md)

## 概要

このドキュメントは、現状維持アプローチ（オプション1）における semantic-release の設定構造を定義します。`.releaserc.json` ファイルを明示的に作成することで、デフォルト設定への暗黙的な依存を排除し、設定の可視化と保守性を向上させます。

## エンティティ定義

### 1. Releaserc Configuration

semantic-release の設定ファイル `.releaserc.json` の構造を定義します。

#### 構造

```typescript
interface ReleasercConfig {
  branches: string[];           // リリース対象ブランチ
  plugins: Plugin[];            // 使用するプラグイン
  tagFormat?: string;           // タグフォーマット（オプション）
  repositoryUrl?: string;       // リポジトリURL（オプション）
}

interface Plugin {
  name: string;                 // プラグイン名
  config?: Record<string, any>; // プラグイン固有の設定
}
```

#### フィールド詳細

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|-----|------|-----------|------|
| `branches` | `string[]` | ✅ | `["main"]` | リリースを実行するブランチのリスト |
| `plugins` | `Plugin[]` | ✅ | デフォルトプラグイン | semantic-release の実行フェーズを定義 |
| `tagFormat` | `string` | ❌ | `"v${version}"` | Git タグのフォーマット（例: v1.0.0） |
| `repositoryUrl` | `string` | ❌ | package.json から自動検出 | リポジトリの URL |

### 2. Plugin Configuration

semantic-release が実行する各プラグインの設定を定義します。

#### 標準プラグイン

| プラグイン名 | フェーズ | 説明 |
|-------------|---------|------|
| `@semantic-release/commit-analyzer` | analyze | コミットメッセージを解析してリリースタイプを決定 |
| `@semantic-release/release-notes-generator` | generate-notes | リリースノートを自動生成 |
| `@semantic-release/changelog` | prepare | CHANGELOG.md を更新 |
| `@semantic-release/npm` | prepare, publish | package.json のバージョンを更新、npm に公開 |
| `@semantic-release/github` | publish, success | GitHub Release を作成 |
| `@semantic-release/git` | prepare | 変更をコミット（CHANGELOG.md, package.json） |

#### プラグイン実行順序

```text
1. analyze (commit-analyzer) → リリースタイプ決定（major/minor/patch）
2. generate-notes (release-notes-generator) → リリースノート生成
3. prepare (changelog, npm, git) → ファイル更新とコミット
4. publish (npm, github) → パッケージ公開とリリース作成
5. success/fail (github) → 成功/失敗通知
```

## 設定ファイル仕様

### `.releaserc.json` の標準設定

プロジェクトのデフォルト動作を明示的に定義した設定：

```json
{
  "branches": ["main"],
  "tagFormat": "v${version}",
  "plugins": [
    "@semantic-release/commit-analyzer",
    "@semantic-release/release-notes-generator",
    [
      "@semantic-release/changelog",
      {
        "changelogFile": "CHANGELOG.md"
      }
    ],
    [
      "@semantic-release/npm",
      {
        "npmPublish": true
      }
    ],
    [
      "@semantic-release/git",
      {
        "assets": ["CHANGELOG.md", "package.json"],
        "message": "chore(release): ${nextRelease.version} [skip ci]\n\n${nextRelease.notes}"
      }
    ],
    "@semantic-release/github"
  ]
}
```

### 設定項目の詳細

#### `branches` フィールド

```json
{
  "branches": ["main"]
}
```

- **説明**: リリースワークフローを実行するブランチ
- **現状**: `main` ブランチのみ
- **動作**: main ブランチへのプッシュで semantic-release が自動実行

#### `tagFormat` フィールド

```json
{
  "tagFormat": "v${version}"
}
```

- **説明**: Git タグのフォーマット
- **例**: バージョン 1.2.3 の場合 → `v1.2.3`
- **互換性**: npm version コマンドと同じフォーマット

#### `plugins` フィールド

各プラグインの役割と設定：

**1. @semantic-release/commit-analyzer**

```json
"@semantic-release/commit-analyzer"
```

- **役割**: コミットメッセージを解析してリリースタイプを決定
- **解析ルール**: Conventional Commits に基づく
  - `feat:` → minor バージョンアップ
  - `fix:` → patch バージョンアップ
  - `BREAKING CHANGE:` → major バージョンアップ

**2. @semantic-release/release-notes-generator**

```json
"@semantic-release/release-notes-generator"
```

- **役割**: コミット履歴からリリースノートを自動生成
- **出力**: GitHub Release の説明文

**3. @semantic-release/changelog**

```json
[
  "@semantic-release/changelog",
  {
    "changelogFile": "CHANGELOG.md"
  }
]
```

- **役割**: CHANGELOG.md ファイルを自動更新
- **設定**: `changelogFile` で出力先を指定

**4. @semantic-release/npm**

```json
[
  "@semantic-release/npm",
  {
    "npmPublish": true
  }
]
```

- **役割**: package.json のバージョン更新と npm への公開
- **設定**: `npmPublish: true` で npm registry に公開

**5. @semantic-release/git**

```json
[
  "@semantic-release/git",
  {
    "assets": ["CHANGELOG.md", "package.json"],
    "message": "chore(release): ${nextRelease.version} [skip ci]\n\n${nextRelease.notes}"
  }
]
```

- **役割**: 変更されたファイルを Git にコミット
- **設定**:
  - `assets`: コミットするファイルのリスト
  - `message`: コミットメッセージのテンプレート
  - `[skip ci]`: CI の無限ループを防ぐ

**6. @semantic-release/github**

```json
"@semantic-release/github"
```

- **役割**: GitHub Release の作成とタグの作成
- **動作**: リリースノートと共に GitHub Release を公開

## 検証ルール

### 1. branches フィールドの検証

- **ルール**: `branches` 配列は空であってはならない
- **ルール**: `main` ブランチが含まれていなければならない
- **エラー例**: `branches: []` → エラー

### 2. plugins フィールドの検証

- **ルール**: `plugins` 配列は空であってはならない
- **ルール**: 最低限以下のプラグインが含まれていなければならない：
  - `@semantic-release/commit-analyzer`
  - `@semantic-release/release-notes-generator`
  - `@semantic-release/npm`
  - `@semantic-release/github`

### 3. tagFormat フィールドの検証

- **ルール**: `${version}` プレースホルダーを含まなければならない
- **推奨**: `v${version}` フォーマット（npm version との互換性）

## 状態遷移

### リリースプロセスの状態遷移

```text
[main ブランチへのプッシュ]
    ↓
[GitHub Actions トリガー]
    ↓
[semantic-release 実行開始]
    ↓
[1. コミット解析] → リリースタイプ決定（major/minor/patch）
    ↓
[2. リリースノート生成] → GitHub Release 用の説明文作成
    ↓
[3. CHANGELOG.md 更新] → ファイル更新
    ↓
[4. package.json 更新] → バージョン番号更新
    ↓
[5. Git コミット] → CHANGELOG.md, package.json をコミット
    ↓
[6. Git タグ作成] → v*.*.* 形式のタグ作成
    ↓
[7. npm publish] → npm registry に公開
    ↓
[8. GitHub Release 作成] → タグとリリースノートを公開
    ↓
[リリース完了]
```

### エラー状態の処理

| エラー状態 | 原因 | 復旧方法 |
|-----------|------|---------|
| No release type | リリース対象のコミットがない | Conventional Commits でコミット |
| npm publish 失敗 | NPM_TOKEN が無効 | GitHub Secrets の NPM_TOKEN を更新 |
| Git コミット失敗 | 権限不足 | GITHUB_TOKEN の権限を確認 |
| タグ作成失敗 | 同じタグが既に存在 | 手動でタグを削除して再実行 |

## 関係性

### ファイル間の依存関係

```text
.releaserc.json
    ↓ (設定を読み込む)
semantic-release (GitHub Actions)
    ↓ (参照)
package.json (バージョン情報、リポジトリ URL)
    ↓ (更新)
CHANGELOG.md, package.json
    ↓ (コミット)
Git リポジトリ (main ブランチ)
    ↓ (タグ作成)
GitHub Release
    ↓ (公開)
npm registry
```

### GitHub Actions との関係

```text
.github/workflows/release.yml
    ↓ (トリガー: on.push.branches: [main])
GitHub Actions Runner
    ↓ (実行)
bunx semantic-release
    ↓ (設定読み込み)
.releaserc.json
    ↓ (環境変数読み込み)
GITHUB_TOKEN, NPM_TOKEN (GitHub Secrets)
```

## 拡張性

### プレリリース版のサポート（将来的な拡張）

プレリリース版（beta, alpha）をサポートする場合の設定例：

```json
{
  "branches": [
    "main",
    {
      "name": "beta",
      "prerelease": true
    },
    {
      "name": "alpha",
      "prerelease": true
    }
  ],
  "tagFormat": "v${version}",
  "plugins": [
    "@semantic-release/commit-analyzer",
    "@semantic-release/release-notes-generator",
    "@semantic-release/changelog",
    "@semantic-release/npm",
    "@semantic-release/git",
    "@semantic-release/github"
  ]
}
```

**動作**:
- `main` ブランチ → `v1.2.3` 形式のタグ
- `beta` ブランチ → `v1.2.3-beta.1` 形式のタグ
- `alpha` ブランチ → `v1.2.3-alpha.1` 形式のタグ

## まとめ

このデータモデルは、semantic-release の設定構造を明確に定義し、以下の目標を達成します：

1. **設定の可視化**: デフォルト設定への暗黙的な依存を排除
2. **保守性の向上**: 設定変更時の影響範囲を明確化
3. **ドキュメント化**: リリースプロセスの理解を促進
4. **拡張性**: 将来的なプレリリース版サポートへの対応を考慮

次のステップ: [quickstart.md](./quickstart.md) でリリースプロセスの実行手順を定義します。
