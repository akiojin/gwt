# 技術調査: GitHub Actions リリースワークフローのタグトリガー変更

**仕様ID**: `SPEC-23bb2eed` | **日付**: 2025-10-25 | **仕様書**: [spec.md](./spec.md)

## 概要

GitHub Actions のリリースワークフローを main ブランチトリガーから v* タグトリガーへ変更する可能性を調査しました。この調査の結果、**タグトリガーへの変更は技術的に不適切**であることが判明しました。

## 調査項目と結果

### 1. GitHub Actions on.push.tags 構文

**調査内容**: v* パターンのタグにマッチする正しい構文を調査

**結果**:
```yaml
on:
  push:
    tags:
      - 'v*'           # v1.0.0, v2.1.3, v1.0.0-beta.1 など
      - '!v*-*'        # プレリリースを除外する場合（オプション）
```

**詳細**:
- `tags: ['v*']` でv接頭辞付きのすべてのタグにマッチ
- glob パターンを使用（正規表現ではない）
- プレリリースタグ（v1.0.0-beta.1など）も自動的に含まれる
- 除外パターン（`!`）で特定タグを除外可能

### 2. npm version コマンドとタグ生成

**調査内容**: npm version コマンドがどのようにタグを生成するか

**結果**:

| コマンド | 生成されるタグ例 | 説明 |
|---------|----------------|------|
| `npm version patch` | v1.0.1 | パッチバージョン更新 |
| `npm version minor` | v1.1.0 | マイナーバージョン更新 |
| `npm version major` | v2.0.0 | メジャーバージョン更新 |
| `npm version prerelease --preid=beta` | v1.0.1-beta.0 | プレリリース版 |

**動作**:
1. package.json のバージョンを更新
2. Git コミットを自動作成
3. v接頭辞付きタグを自動作成（ローカル）
4. `git push --tags` でリモートにプッシュ

### 3. semantic-release とタグトリガーの互換性（重要）

**調査内容**: semantic-release がタグトリガーで正常に動作するか

**結果**: **semantic-release とタグトリガーは互換性がありません**

**理由**:

1. **循環依存の問題**
   - semantic-release 自体がタグを作成するツール
   - タグトリガーで実行すると、タグが既に存在するため semantic-release が正常に動作しない
   - semantic-release は「タグを作成する前」に実行される設計

2. **GITHUB_TOKEN の制限**
   - デフォルトの GITHUB_TOKEN で作成されたタグは新しいワークフローをトリガーしない
   - Personal Access Token (PAT) が必要（セキュリティリスク）

3. **自動化機能の喪失**
   - Conventional Commits の自動解析が不可
   - バージョン自動決定が不可
   - CHANGELOG 自動生成が不可
   - GitHub Release 自動作成が影響を受ける

**semantic-release の推奨トリガー**:
```yaml
on:
  push:
    branches: [main]
```

### 4. 既存のリリースワークフローとの互換性

**調査内容**: 現在の release.yml の処理と変更の影響

**現在の構成**:
```yaml
on:
  push:
    branches: [main]

jobs:
  release:
    steps:
      - テスト実行
      - ビルド実行
      - semantic-release 実行（自動バージョン決定、タグ作成、リリース作成）
      - npm publish 実行
```

**タグトリガーに変更した場合の影響**:

| 項目 | 現状（main トリガー） | タグトリガー後 |
|-----|---------------------|--------------|
| バージョン決定 | 自動（Conventional Commits） | 手動（npm version） |
| タグ作成 | 自動（semantic-release） | 手動（npm version） |
| CHANGELOG 生成 | 自動 | 手動または不可 |
| GitHub Release | 自動 | 手動設定が必要 |
| npm publish | 自動 | 自動（維持可能） |
| 開発者の作業 | PR マージのみ | npm version + git push --tags |

## 技術的決定

### 決定: 現状の main ブランチトリガーを維持（仕様変更を推奨）

**根拠**:

1. **semantic-release との完全互換性**
   - semantic-release は main ブランチトリガーで動作するように設計されている
   - 自動バージョニング、タグ作成、リリースノート生成が統合されている

2. **プロジェクト原則との整合性**
   - CLAUDE.md: "設計・実装は複雑にせずに、シンプルさの極限を追求"
   - 現状の自動化が最もシンプルで効率的

3. **開発者体験の維持**
   - PR をマージするだけで自動的にリリース
   - 手動操作不要、ヒューマンエラーのリスク低減

4. **業界標準のベストプラクティス**
   - semantic-release は広く採用されている標準ツール
   - GitHub Actions + semantic-release の組み合わせが推奨パターン

### 検討した代替案

#### 代替案1: 手動タグ作成 + タグトリガー

**アプローチ**:
```yaml
on:
  push:
    tags: ['v*']

jobs:
  release:
    steps:
      - テスト、ビルド
      - GitHub Release 作成（手動）
      - npm publish
```

**メリット**:
- リリースタイミングの明示的コントロール
- semantic-release への依存排除

**デメリット**:
- Conventional Commits の自動解析機能喪失
- CHANGELOG の手動メンテナンス必要
- バージョン決定の手動化（ヒューマンエラーリスク）
- リリースノートの手動作成
- 開発者の作業負荷増加

**結論**: **採用しない**（シンプルさと自動化の原則に反する）

#### 代替案2: npm version + タグトリガー（semantic-release 削除）

**アプローチ**:
```bash
# 開発者の操作
npm version patch
git push --tags
```

**メリット**:
- semver に完全準拠
- タグとバージョンの完全同期

**デメリット**:
- semantic-release の全機能喪失
- Conventional Commits 活用不可
- CHANGELOG の手動更新
- リリースノートの手動作成
- 手動操作の増加

**結論**: **採用しない**（自動化機能の喪失が大きすぎる）

#### 代替案3: ハイブリッドアプローチ（2段階ワークフロー）

**アプローチ**:
```yaml
# ワークフロー1: semantic-release でタグ作成（main トリガー）
on:
  push:
    branches: [main]
jobs:
  create-release:
    - semantic-release（タグ作成のみ）

# ワークフロー2: npm publish（タグトリガー）
on:
  push:
    tags: ['v*']
jobs:
  publish:
    - npm publish
```

**メリット**:
- semantic-release の自動化機能維持
- タグトリガーの活用

**デメリット**:
- ワークフロー複雑化
- Personal Access Token (PAT) が必要（セキュリティリスク）
- メンテナンス負荷増加
- デバッグ困難

**結論**: **採用しない**（複雑さがシンプルさの原則に反する）

## リスク評価

### 現状維持（main ブランチトリガー）のリスク

**リスク**: 意図しないリリースが実行される可能性

**リスクレベル**: **低**

**緩和策**:
1. ブランチプロテクション設定（main への直接プッシュ禁止）
2. PR レビュープロセスの徹底
3. Conventional Commits の遵守（feat:, fix: など）
4. テストの自動実行（リリース前に品質確保）

**既存の対策**:
- release.yml でテスト実行（line 33-34）
- ビルド検証（line 36-45）
- PR ベースの開発フロー

### タグトリガー移行のリスク

**リスク**: 自動化機能の喪失と手動作業の増加

**リスクレベル**: **高**

**影響**:
1. semantic-release 機能の完全喪失
2. CHANGELOG の手動更新が必要
3. リリースノートの手動作成が必要
4. バージョン決定の手動化（ヒューマンエラーリスク）
5. 開発者の作業負荷増加
6. リリースプロセスの複雑化

**結論**: リスクに対するリターンが低い

## 推奨される実装アプローチ

### オプション1: 仕様を変更して現状維持（推奨）

**実装内容**:
1. `.releaserc.json` ファイルを追加してデフォルト設定への依存を排除
2. リリースプロセスを README.md に明確に記載
3. ブランチプロテクション設定の強化（該当する場合）

**メリット**:
- 既存の自動化機能を完全に維持
- 最もシンプルで信頼性の高いアプローチ
- semantic-release のベストプラクティスに準拠
- 開発者体験が最高

**変更範囲**: 最小限（ドキュメントと設定の明示化のみ）

### オプション2: semantic-release を削除してタグトリガーに移行（非推奨）

**実装内容**:
1. semantic-release を package.json から削除
2. release.yml を v* タグトリガーに変更
3. GitHub Release 作成ステップを手動設定で追加
4. CHANGELOG 更新プロセスを手動化

**メリット**:
- ユーザーの元の要望に完全に対応

**デメリット**:
- 自動化機能の完全喪失
- 開発者の作業負荷が大幅に増加
- CLAUDE.md のシンプルさの原則に反する
- メンテナンス負荷増加

**変更範囲**: 大規模（ワークフロー全体の再設計）

## 参考資料

1. [GitHub Actions - Workflow syntax for GitHub Actions](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
2. [semantic-release - Usage](https://semantic-release.gitbook.io/semantic-release/usage/ci-configuration)
3. [npm version - Documentation](https://docs.npmjs.com/cli/version)
4. [Conventional Commits](https://www.conventionalcommits.org/)

## 次のステップ

1. **ユーザーへの報告**: 調査結果を報告し、仕様変更の判断を仰ぐ
2. **仕様の更新**: ユーザーの決定に基づいて spec.md を更新
3. **設計フェーズへ**: 決定されたアプローチに基づいて Phase 1（設計）へ進む

## 結論

**技術調査の結論**: タグトリガーへの変更は semantic-release との互換性がないため、技術的に不適切です。現状の main ブランチトリガーを維持することを強く推奨します。

ユーザーの元の要望（タグトリガーへの変更）を実現するには、semantic-release を削除して手動リリースプロセスに移行する必要がありますが、これはプロジェクトの「シンプルさの極限を追求」という原則に反します。

**推奨**: 仕様を見直し、現状の自動化機能を維持する方向で進めることを提案します。
