# 技術調査: GitHub Actions タグトリガーとリリースワークフロー最適化

**仕様ID**: `SPEC-473b3d47` | **日付**: 2025-10-25
**関連ドキュメント**: [spec.md](./spec.md) | [plan.md](./plan.md)

## 調査概要

このドキュメントは、GitHub Actions のタグトリガーに関する調査結果をまとめています。現在の main ブランチトリガーから、タグベースのリリースワークフローへの移行可能性と、その影響について検証します。

## 調査項目1: GitHub Actions on.push.tags 構文

### v* パターンのタグマッチング

**基本構文**:
```yaml
on:
  push:
    tags:
      - 'v*'  # v1.0, v20.15.10 などにマッチ
```

**重要な仕様**:
- GitHub Actions は glob パターンを使用（正規表現ではない）
- `*` は任意の文字列にマッチ
- `**` は階層的パターンにマッチ
- 正規表現構文（`[0-9]+`, `()` など）は使用不可

### 複数タグパターンの指定

**複数パターン例**:
```yaml
on:
  push:
    tags:
      - 'v*'           # すべての v で始まるタグ
      - 'release-*'    # release- で始まるタグ
```

**除外パターンの使用**:
```yaml
on:
  push:
    tags:
      - 'v*'           # すべての v で始まるタグ
      - '!v*-pre'      # プレリリースタグを除外
      - '!v*-alpha'    # アルファタグを除外
      - '!v*-beta'     # ベータタグを除外
```

**重要な制約**:
- `tags` と `tags-ignore` を同一イベントで併用不可
- 除外パターン（`!` プレフィックス）を使用する場合、最低1つの包含パターンが必要
- パターンの順序が重要（包含パターン → 除外パターンの順）

### プレリリースタグの扱い

**プレリリースタグフォーマット例**:
- `v1.0.0-beta.1`
- `v2.1.0-alpha.3`
- `v1.5.0-rc.2`

**マッチング動作**:
- `v*` パターンは **すべてのプレリリースタグを含む**
- プレリリースを除外する場合、明示的な除外パターンが必要
- semver 準拠のタグすべてに `v*` はマッチする

**推奨パターン（プレリリース除外）**:
```yaml
on:
  push:
    tags:
      - 'v[0-9]+.[0-9]+.[0-9]+'        # 正式リリースのみ
      - '!v*-*'                         # ハイフン含むタグを除外
```

ただし、glob パターンでは `[0-9]+` は利用不可のため、実際には:
```yaml
on:
  push:
    tags:
      - 'v*'           # すべての v タグ
      - '!v*-*'        # ハイフン含むタグを除外
```

## 調査項目2: npm version コマンドとタグの関係

### 標準バージョンコマンド

**基本的な動作**:
```bash
npm version patch    # 1.0.0 → 1.0.1 (タグ: v1.0.1)
npm version minor    # 1.0.0 → 1.1.0 (タグ: v1.1.0)
npm version major    # 1.0.0 → 2.0.0 (タグ: v2.0.0)
```

**タグフォーマット**:
- デフォルトで `v` プレフィックスが付与される
- Git タグが自動作成される
- コミットも自動作成される

**タグプレフィックスのカスタマイズ**:
```bash
# .npmrc ファイルで設定
npm config set tag-version-prefix 'custom-'

# プレフィックスなし
npm --no-git-tag-version version patch
```

### プレリリースバージョンコマンド

**プレリリースコマンド**:
```bash
npm version premajor --preid=rc     # 1.0.0 → 2.0.0-rc.0
npm version preminor --preid=beta   # 1.0.0 → 1.1.0-beta.0
npm version prepatch --preid=alpha  # 1.0.0 → 1.0.1-alpha.0
npm version prerelease              # 1.0.1-alpha.0 → 1.0.1-alpha.1
```

**タグフォーマット例**:
- `v2.0.0-rc.0`
- `v1.1.0-beta.0`
- `v1.0.1-alpha.0`, `v1.0.1-alpha.1`, `v1.0.1-alpha.2` ...

**npm publish との連携**:
```bash
npm publish --tag next    # latest タグではなく next タグで公開
npm publish --tag beta    # beta タグで公開
```

### git push --tags の動作

**自動タグプッシュ**:
```bash
npm version patch    # ローカルでタグ作成
git push --tags      # すべてのタグをリモートにプッシュ
```

**個別タグプッシュ**:
```bash
git push origin v1.0.1    # 特定のタグのみプッシュ
```

**重要な注意点**:
- `npm version` はローカルでタグを作成するのみ
- リモートへのプッシュは手動または CI で実施
- `git push --tags` はすべてのローカルタグをプッシュ（注意が必要）

## 調査項目3: semantic-release とタグトリガーの互換性

### semantic-release の標準動作

**推奨トリガー: main ブランチプッシュ**
```yaml
on:
  push:
    branches:
      - main
```

**動作フロー**:
1. main ブランチにコミットがプッシュされる
2. semantic-release がコミットメッセージを解析
3. 新しいバージョンを決定
4. タグを作成
5. GitHub リリースを作成
6. npm に公開

### タグトリガーとの互換性問題

**重要な発見: タグトリガーは semantic-release に適さない**

**理由**:
1. **循環依存の問題**: semantic-release 自身がタグを作成するため、タグトリガーで semantic-release を実行すると循環参照となる
2. **GITHUB_TOKEN の制限**: デフォルトの `GITHUB_TOKEN` で作成されたタグは、新しいワークフローをトリガーしない（無限ループ防止）
3. **[skip ci] の影響**: semantic-release が作成するリリースコミットにはデフォルトで `[skip ci]` が含まれる

### タグから自動バージョン検出

**semantic-release の仕組み**:
- Git履歴からバージョンを決定（タグから直接読み取るのではない）
- コミットメッセージの Conventional Commits 形式を解析
- 前回のリリースタグから現在までのコミットを分析

**タグトリガーでの実行は不可**:
- semantic-release はタグ作成**前**に実行される必要がある
- 既存のタグに対して実行しても意味がない

### main ブランチトリガーからの移行

**結論: 移行は推奨されない**

**理由**:
1. semantic-release のデザイン原則に反する
2. タグは semantic-release の出力であり、入力ではない
3. 追加の複雑性とリスクが高い

**代替案**:
- main ブランチトリガーを維持（現状維持）
- 手動リリースプロセスに完全移行（semantic-release を使用しない）

## 調査項目4: 既存のリリースワークフローとの互換性

### 現在の release.yml の処理内容

**ワークフロー構成**:
```yaml
on:
  push:
    branches: [main]

jobs:
  release:
    steps:
      - Checkout code
      - Setup Bun
      - Install dependencies
      - Run tests
      - Build
      - Verify build
      - Semantic Release (タグ作成、GitHub リリース作成)

  publish-npm:
    needs: release
    steps:
      - Checkout code
      - Setup Bun
      - Install dependencies
      - Build
      - Publish to npm
```

**現在の動作フロー**:
1. main ブランチへのプッシュでトリガー
2. テストとビルドを実行
3. semantic-release が自動的に:
   - バージョン決定
   - タグ作成
   - GitHub リリース作成
4. npm への公開（独立ジョブ）

### タグトリガーに変更した場合の影響

**影響分析**:

1. **semantic-release の実行不可**:
   - タグは既に作成済み（手動または別プロセス）
   - semantic-release は何もすることがない
   - 代わりに GitHub Release 作成のみを行う必要がある

2. **ワークフロー分離の必要性**:
   - タグ作成ワークフロー（手動または自動）
   - リリース作成ワークフロー（タグトリガー）
   - npm 公開ワークフロー（タグトリガー）

3. **テストのタイミング**:
   - 現状: リリース前にテスト実行
   - タグトリガー: タグ作成前にテスト済みである必要
   - → PR マージ前のテストが重要に

**変更が必要な箇所**:
```yaml
# 変更前
on:
  push:
    branches: [main]

jobs:
  release:
    steps:
      - name: Semantic Release
        run: node node_modules/semantic-release/bin/semantic-release.js

# 変更後（タグトリガーの場合）
on:
  push:
    tags:
      - 'v*'
      - '!v*-*'  # プレリリース除外

jobs:
  release:
    steps:
      - name: Create GitHub Release
        uses: actions/create-release@v1
        with:
          tag_name: ${{ github.ref_name }}
          # semantic-release は使用しない
```

### 必要な設定変更

**タグトリガー移行時の変更リスト**:

1. **タグ作成プロセスの確立**:
   - 手動: `git tag v1.0.0 && git push origin v1.0.0`
   - 自動: 別ワークフローで `npm version` 使用
   - 問題: Conventional Commits の自動解析が失われる

2. **GitHub Release 作成の代替**:
   - semantic-release の `@semantic-release/github` の代わりに
   - `actions/create-release` または `gh release create` を使用

3. **npm 公開の調整**:
   - タグからバージョンを抽出
   - `package.json` の version フィールドを更新

4. **リリースノート生成**:
   - semantic-release の自動生成機能が使えない
   - 代替: `github-changelog-generator` や手動作成

## 技術的決定のサマリー

### 決定1: 現状の main ブランチトリガーを維持

**決定**: タグトリガーへの移行は行わない

**根拠**:
1. semantic-release のデザインと完全に互換性がある
2. 自動バージョニング、タグ作成、リリースノート生成が統合されている
3. プロジェクトの「シンプルさの極限を追求」原則に合致
4. 開発者体験が最高（手動タグ作成不要）

### 決定2: プレリリースサポートの追加検討

**決定**: 必要に応じて beta/alpha ブランチを追加可能

**構成例**:
```yaml
on:
  push:
    branches:
      - main
      - beta
      - alpha
```

**semantic-release 設定**:
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
  ]
}
```

**動作**:
- `main`: 正式リリース（v1.0.0）
- `beta`: ベータリリース（v2.0.0-beta.1）
- `alpha`: アルファリリース（v2.0.0-alpha.1）

### 決定3: 既存ワークフローの最適化

**決定**: semantic-release 設定ファイルを明示的に追加

**理由**:
- デフォルト設定に依存しない
- プレリリースブランチのカスタマイズが可能
- CI/CD の透明性向上

**推奨ファイル**: `.releaserc.json`
```json
{
  "branches": ["main"],
  "plugins": [
    "@semantic-release/commit-analyzer",
    "@semantic-release/release-notes-generator",
    "@semantic-release/changelog",
    "@semantic-release/npm",
    "@semantic-release/github",
    "@semantic-release/git"
  ]
}
```

### 決定4: ドキュメント更新不要

**決定**: 現状のワークフローで問題なし

**理由**:
- タグトリガーへの移行を行わないため
- 既存の release.yml は適切に動作している
- 追加のドキュメント作成は不要

## 検討した代替案

### 代替案1: 手動タグ作成 + タグトリガーリリース

**概要**:
1. 開発者が手動で `git tag v1.0.0` を作成
2. タグプッシュで GitHub Actions がトリガー
3. テスト、ビルド、npm 公開を実行

**メリット**:
- 明示的なリリースコントロール
- semantic-release の依存関係削除

**デメリット**:
- バージョン決定が手動（ヒューマンエラーのリスク）
- リリースノート自動生成の喪失
- Conventional Commits の活用不可
- 開発者体験の低下

**結論**: 採用しない

### 代替案2: npm version コマンド + タグトリガー

**概要**:
1. 開発者が `npm version patch/minor/major` を実行
2. 自動作成されたタグをプッシュ
3. GitHub Actions でリリース処理

**メリット**:
- semver に準拠したバージョニング
- タグとバージョンの同期が保証

**デメリット**:
- Conventional Commits の自動解析なし
- リリースノート生成は別途必要
- semantic-release より手動操作が増加

**結論**: 採用しない

### 代替案3: ハイブリッドアプローチ

**概要**:
1. main ブランチトリガーで semantic-release 実行（タグ作成のみ）
2. タグトリガーで npm 公開ワークフローを実行

**メリット**:
- semantic-release の自動化を維持
- npm 公開を分離できる

**デメリット**:
- ワークフローの複雑化
- GITHUB_TOKEN の制限により、タグトリガーが発火しない可能性
- PAT（Personal Access Token）が必要になる

**結論**: 採用しない（複雑性が増加）

## リスク評価

### 現状維持（main ブランチトリガー）のリスク: 低

**リスク**:
1. main ブランチへの直接プッシュでリリースが発生
2. 意図しないリリースの可能性

**緩和策**:
- ブランチプロテクションルールの設定
- PR マージフローの徹底
- CI でのテストゲート

### タグトリガー移行のリスク: 高

**リスク**:
1. semantic-release の機能喪失
2. 手動プロセスの増加によるヒューマンエラー
3. ワークフロー複雑化によるメンテナンス負荷
4. リリースノート生成の代替手段が必要

**緩和策**:
- 詳細なドキュメント作成
- リリース手順のチェックリスト化
- 代替ツールの導入と検証

**結論**: リスクに対するリターンが低い

## npm version と semantic-release の比較

| 項目 | npm version | semantic-release |
|------|-------------|------------------|
| バージョン決定 | 手動（patch/minor/major指定） | 自動（コミットメッセージ解析） |
| タグ作成 | 自動（v プレフィックス） | 自動（カスタマイズ可能） |
| リリースノート | 手動作成 | 自動生成 |
| GitHub リリース | 手動または別ツール | 自動作成 |
| npm 公開 | 手動実行 | 自動実行 |
| CI/CD 統合 | 要追加実装 | ネイティブサポート |
| プレリリース | preid 指定で可能 | ブランチ設定で自動 |
| 学習コスト | 低 | 中 |
| 自動化レベル | 低 | 高 |

**結論**: semantic-release が本プロジェクトに適している

## 次のステップ

### 推奨アクション

1. **現状維持**: main ブランチトリガーを継続使用
2. **設定明示化**: `.releaserc.json` ファイルを追加
3. **ドキュメント整備**: リリースプロセスを README に記載
4. **プレリリース検討**: 必要に応じて beta/alpha ブランチを追加

### 将来的な拡張オプション

**プレリリースブランチの追加**:
```yaml
# .releaserc.json
{
  "branches": [
    "main",
    {"name": "beta", "prerelease": true}
  ]
}
```

**カスタムリリースノート**:
```yaml
# .releaserc.json に追加
{
  "plugins": [
    ["@semantic-release/release-notes-generator", {
      "preset": "conventionalcommits"
    }]
  ]
}
```

### 実装不要な項目

- タグトリガーへの移行
- 手動タグ作成プロセス
- npm version コマンドの統合
- 代替リリースノート生成ツール

## 結論

**最終的な技術的決定**:

1. **GitHub Actions のタグトリガーは、本プロジェクトには適さない**
   - semantic-release との互換性問題
   - 開発者体験の低下
   - 複雑性の増加

2. **現在の main ブランチトリガーを維持する**
   - semantic-release の全機能を活用
   - 自動バージョニング、リリースノート生成
   - シンプルで保守性が高い

3. **必要な改善: 設定の明示化**
   - `.releaserc.json` ファイルを追加
   - デフォルト設定への依存を排除
   - 将来的な拡張性を確保

4. **タグトリガーが適したユースケース**:
   - semantic-release を使用しないプロジェクト
   - 手動リリースプロセスを採用
   - バージョン管理を厳密にコントロールしたい場合

本プロジェクトは「シンプルさの極限を追求」しつつ「開発者体験の品質は決して妥協しない」方針であり、semantic-release + main ブランチトリガーの組み合わせが最適解である。
