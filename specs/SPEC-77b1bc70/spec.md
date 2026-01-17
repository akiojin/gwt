# 機能仕様: リリースフロー要件の明文化とリリース開始時 main→develop 同期

**仕様ID**: `SPEC-77b1bc70`
**作成日**: 2026-01-16
**ステータス**: ドラフト
**入力**: ユーザー説明: "リリースガイドは不要で、それらを要件化して下さい。その上で、今回の修正をして下さい。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - リリース開始から公開までを自動化したい (優先度: P1)

メンテナは develop に変更を集約し、/release を実行するだけで
バージョン判定・CHANGELOG 更新・release PR 作成・公開まで一連の処理が完了する状態にしたい。

**この優先度の理由**: リリース作業の属人化を防ぎ、公開漏れや手順抜けを排除するため最優先。

**独立したテスト**: prepare-release workflow 実行後に release PR が作成され、main へのマージ後に release.yml が公開まで完了することを確認できれば十分。

**受け入れシナリオ**:

1. **前提条件**: develop にリリース対象のコミットがある、**操作**: prepare-release workflow を実行する、**期待結果**: バージョン判定・CHANGELOG 更新・release PR 作成が完了する
2. **前提条件**: release PR が main にマージされる、**操作**: release.yml を実行する、**期待結果**: タグ・GitHub Release 作成、バイナリアップロード、npm 公開が完了する

---

### ユーザーストーリー 2 - リリース開始時に main を develop に統合したい (優先度: P1)

リリース開始時点で main の最新変更が develop に取り込まれている状態にしたい。
これにより hotfix など main 側の変更がリリース候補から漏れるのを防ぐ。

**この優先度の理由**: 重要な修正が配布に含まれないリスクを避けるため最優先。

**独立したテスト**: prepare-release workflow 実行後に develop が main の最新コミットを含むことを確認できれば十分。

**受け入れシナリオ**:

1. **前提条件**: main に develop へ未反映のコミットがある、**操作**: prepare-release workflow を実行する、**期待結果**: develop に main のコミットが `--no-ff` マージで統合される
2. **前提条件**: develop が main と同一、**操作**: prepare-release workflow を実行する、**期待結果**: 差分がなくても処理が成功する

---

### ユーザーストーリー 3 - リリースガイドを廃止し、仕様へ統合したい (優先度: P2)

重複ドキュメントを減らすため、リリースガイドを削除し、必要な情報は仕様書に集約したい。

**この優先度の理由**: ガイドと実装の乖離が発生しやすく、更新漏れを避けたい。

**独立したテスト**: release-guide.md / release-guide.ja.md が削除され、仕様書内にリリース要件が記載されていることを確認できれば十分。

**受け入れシナリオ**:

1. **前提条件**: リポジトリに release-guide が存在する、**操作**: 仕様更新を行う、**期待結果**: ガイドが削除され、仕様書に要件が明記される

### エッジケース

- main→develop の統合時に競合が発生した場合、workflow は失敗し、release PR は作成されない
- トークン権限不足で develop へ push できない場合、workflow は失敗する

## 要件 *(必須)*

### 機能要件

- **FR-001**: prepare-release workflow は開始時に main を develop に統合**しなければならない**
- **FR-002**: main 統合に失敗した場合、workflow は失敗し、release PR 作成を行っては**ならない**
- **FR-003**: main 統合が成功した場合、develop を origin へ push **しなければならない**
- **FR-004**: prepare-release workflow は Conventional Commits を解析し、バージョンを自動判定**しなければならない**
- **FR-005**: prepare-release workflow は git-cliff により CHANGELOG.md を更新**しなければならない**
- **FR-006**: prepare-release workflow は Cargo.toml と package.json のバージョンを更新**しなければならない**
- **FR-007**: prepare-release workflow は release/YYYYMMDD-HHMMSS ブランチから main への release PR を作成**しなければならない**
- **FR-008**: release.yml はタグと GitHub Release を作成**しなければならない**
- **FR-009**: release.yml はクロスコンパイル済みバイナリを GitHub Release にアップロード**しなければならない**
- **FR-010**: release.yml は npm へ provenance 付きで公開**しなければならない**
- **FR-011**: release.yml から main→develop の back-merge（sync-develop）処理を削除**しなければならない**
- **FR-012**: release-guide.md / release-guide.ja.md を削除**しなければならない**
- **FR-013**: prepare-release workflow は `GITHUB_TOKEN` を使って develop へ push **しなければならない**

### 主要エンティティ *(機能がデータを含む場合は含める)*

- なし

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: prepare-release workflow 実行後、develop が main の最新コミットを含む
- **SC-002**: prepare-release workflow 実行後、release PR が作成される
- **SC-003**: release.yml 実行後、`vX.Y.Z` タグと GitHub Release が作成される
- **SC-004**: release.yml 実行後、GitHub Release にバイナリが添付される
- **SC-005**: release.yml 実行後、npm に新バージョンが公開される
- **SC-006**: release.yml に sync-develop ジョブが存在しない
- **SC-007**: release-guide.md / release-guide.ja.md が削除されている

## 制約と仮定 *(該当する場合)*

### 制約

- GitHub Actions で develop への push 権限が必要
- 既存のリリース手順（develop→main の release PR 作成）は維持する
- main→develop の統合は `git merge --no-ff origin/main` で行う

### 仮定

- リリース開始は prepare-release workflow（workflow_dispatch）で行う
- main への直接 hotfix が発生し得るため、リリース開始前に統合する価値がある

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- リリース判定ルール（Conventional Commits）の変更
- ビルド・配布ジョブ（crates.io/npm/GitHub Release）の削除や配布先の変更
- main への直接リリース手順の変更

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- Workflow トークンに contents: write 権限が必要

## 依存関係 *(該当する場合)*

- GitHub Actions の workflow 設定

## 参考資料 *(該当する場合)*

- `.github/workflows/prepare-release.yml`
- `.github/workflows/release.yml`
