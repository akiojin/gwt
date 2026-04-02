# SPEC カタログ監査レポート

更新日: 2026-04-02
対象: `specs/SPEC-*` 41件（open 33 / closed 8）

## 概要

この監査の目的は、ローカル `SPEC` 群を今後継続運用できる形に正規化することです。
今回の結論は次のとおりです。

- タイトル規約が混在している
  - 日本語、英語、`Feature Specification:`、`機能仕様:`、`gwt-spec:` が混在
- `metadata.id` の表記が混在している
  - `1776` と `SPEC-1777` が混在
- `open` のまま planning artifact が不足している `SPEC` が多い
  - `spec-only` の open が 8 件
  - `plan.md` が `TBD` の open が 6 件
- 完了状態の整合性が崩れている
  - open なのに tasks 完了が 5 件
  - closed なのに tasks 未完了が 7 件
- 設計の正本が複数層で衝突している
  - TUI 全体: `SPEC-1541` / `SPEC-1770` / `SPEC-1776`
  - GitHub / Issue / search: `SPEC-1354` / `SPEC-1643` / `SPEC-1714` / `SPEC-1775`
  - workflow: `SPEC-1579` / `SPEC-1787`
  - Docker: `SPEC-1552` / `SPEC-1642`
  - agent runtime: `SPEC-1636` / `SPEC-1646` / `SPEC-1779`

## 正規化ルール

### タイトル

- 原則は日本語ベース
- 形式は `機能名` または `機能名 — 目的/責務`
- `Feature Specification:` / `機能仕様:` / `gwt-spec:` はタイトルから外し、必要なら本文に残す
- 親 `SPEC` との関係はタイトルではなく `Parent:` や本文先頭で表現する

### metadata.json

- `id` は `1776` のような数値文字列で統一する
- `status` は artifact と tasks の実態に合わせる
- `closed` は「その仕様の更新を止める」ことを意味し、未完了 task が残る場合はその理由を明記する

### spec.md

原則として以下の見出しを揃える。

- `Background`
- `User Stories`
- `Acceptance Scenarios`
- `Edge Cases`
- `Functional Requirements`
- `Success Criteria`

旧テンプレートの `SPEC` は、内容を変えずに見出しだけでも揃える。

### status の扱い

- `open + spec-only`: そのまま放置しない。`plan/tasks` を作るか、親 `SPEC` に吸収して閉じる
- `open + tasks 完了`: `close` 候補として再判定する
- `closed + tasks 未完了`: 履歴 `SPEC` として残すなら「tasks は履歴メモであり完了基準ではない」と明記する

## 正本設計

| 領域 | canonical SPEC | 補助 SPEC | 監査判断 |
|---|---|---|---|
| gwt-spec workflow | `SPEC-1579` | `SPEC-1438`, `SPEC-1786`, `SPEC-1787` | `1579` を workflow 正本に固定する |
| workspace 初期化と作業導線 | `SPEC-1787` | `SPEC-1647`, `SPEC-1776` | `1647` は superseded、`1787` を現行正本にする |
| TUI 移行全体 | `SPEC-1776` | `SPEC-1541`, `SPEC-1654`, `SPEC-1768`, `SPEC-1770` | `1776` は親 `SPEC` に限定し、個別機能は子 `SPEC` へ出す |
| terminal emulation | `SPEC-1541` | `SPEC-1770`, `SPEC-1776` | `1541` は vt100/ANSI/resize に限定する |
| workspace shell / tab lifecycle | `SPEC-1654` | `SPEC-1636`, `SPEC-1648`, `SPEC-1776` | `1654` を Shell/Agent/管理画面の正本にする |
| GitHub discovery/search/version history | `SPEC-1643` | `SPEC-1354`, `SPEC-1714`, `SPEC-1775`, `SPEC-1784` | `1643` を discovery/search 正本にする |
| Issue detail rendering | `SPEC-1354` | `SPEC-1643`, `SPEC-1714` | detail view の正本は `1354` に固定する |
| Issue cache / linkage | `SPEC-1714` | `SPEC-1643`, `SPEC-1354` | linkage/caching は `1714` に限定する |
| Docker runtime support | `SPEC-1552` | `SPEC-1642` | `1552` を基盤、`1642` を UI/導線に縮小する |
| agent catalog / runtime | `SPEC-1646` | `SPEC-1636`, `SPEC-1779` | `1646` は検出/起動/バージョンに限定する |
| assistant PTY interaction | `SPEC-1636` | `SPEC-1654` | Assistant の送信/割り込み/queue は `1636` に限定する |
| custom agent registration | `SPEC-1779` | `SPEC-1646` | 設定 UI と登録機能は `1779` に分離維持する |

## SPEC 一覧と是正方針

### 1. 基盤・ランタイム

| SPEC | 推奨タイトル | 判定 | 次アクション |
|---|---|---|---|
| `SPEC-1540` | PTY 管理基盤 | closed / 履歴不整合 | closed のまま残し、tasks は履歴メモであることを明記する |
| `SPEC-1541` | TUI ターミナルエミュレーション | open / 責務肥大 | vt100/ANSI/resize/selection に限定し、入力 UX は `1770` / `1776` へ委譲する |
| `SPEC-1542` | データ永続化レイヤー | closed / 整合 | タイトルと見出しだけ正規化し、保存先の正本として参照元を統一する |
| `SPEC-1543` | Git 操作レイヤー | closed / 履歴不整合 | `1644` との境界を明記し、legacy 基盤として履歴化する |
| `SPEC-1544` | GitHub 連携基盤 | closed / 履歴不整合 | `1643` / `1714` / `1775` へ分割済みであることを明記し、履歴 `SPEC` にする |
| `SPEC-1552` | Docker / DevContainer 対応 | closed / 履歴不整合 | Docker runtime support の正本として残し、`1642` には UI 導線だけを持たせる |

### 2. Workflow / gwt-spec

| SPEC | 推奨タイトル | 判定 | 次アクション |
|---|---|---|---|
| `SPEC-1438` | スキル埋め込みと対象 Worktree への登録 | closed / legacy | `1579` / `1786` に継承済みであることを明記し、履歴 `SPEC` にする |
| `SPEC-1577` | Assistant 組み込みツールシステム | open / 未着手多い | `1579` との責務境界を明記し、進捗管理を追加する |
| `SPEC-1579` | gwt-spec ワークフロー・ストレージ・完了ゲート | open / canonical | workflow 正本として維持し、他 workflow `SPEC` から重複要件を削る |
| `SPEC-1786` | gwt-spec の hooks.json マージ | open / 完了間近 | タイトルと `id` を正規化し、最終 task 完了後に close する |
| `SPEC-1787` | ワークスペース初期化と SPEC 駆動ワークフロー | open / canonical | `1647` superseded を明記し、`1579` とは frontend workflow 境界に限定する |

### 3. Workspace / Session / TUI

| SPEC | 推奨タイトル | 判定 | 次アクション |
|---|---|---|---|
| `SPEC-1636` | Assistant PTY モードの割り込み送信とキュー | open / tasks 完了 | close 候補。残差分がある場合は `1654` 配下の子 `SPEC` に切り出す |
| `SPEC-1644` | ローカル Git バックエンドドメイン | open / tasks 完了 | close 候補。`1543` は履歴 `SPEC` として参照だけ残す |
| `SPEC-1646` | エージェント検出・起動・ライフサイクル | open / `TBD` plan | `1636` と `1779` を除いた runtime/catalog のみに縮小して plan を作り直す |
| `SPEC-1648` | セッション保存・復元 | open / `TBD` plan | `1542` / `1654` / `1776` と境界を切り、session restore 契約に限定して plan を作り直す |
| `SPEC-1654` | ワークスペースシェル | open / 親SPEC | Shell/Agent タブと管理画面の正本として維持し、子 `SPEC` 参照を増やす |
| `SPEC-1768` | タイルシステム共通仕様 | open / 大型SPEC | `1776` 配下の cross-cutting 基盤として残し、子 `SPEC` 参照を整理する |
| `SPEC-1769` | TUI 画像表示（sixel/kitty） | open / 大型SPEC | `1768` の子 `SPEC` として位置付け、tasks を現実的な粒度に再分割する |
| `SPEC-1770` | TUI マウス・キーボード操作 | open / 大型SPEC | interaction 正本として維持し、`1777` / `1780` / `1783` と境界を明記する |
| `SPEC-1776` | Tauri GUI から ratatui TUI への移行 | open / 移行親SPEC | 親 `SPEC` に限定し、詳細機能の追加先は子 `SPEC` に寄せる |
| `SPEC-1777` | SPECs タブ — 一覧・詳細・検索 | open / spec-only | 実装済み機能を基準に plan/tasks を backfill し、子 `SPEC` として close 候補にする |
| `SPEC-1778` | 音声入力 | open / spec-only | 実装済み runtime/設定の実態に合わせて plan/tasks を backfill する |
| `SPEC-1779` | カスタムエージェント登録 | open / spec-only | `1646` の子 `SPEC` として plan/tasks を backfill し、完了後 close 候補にする |
| `SPEC-1780` | ファイル貼り付け（クリップボードから PTY へ） | open / spec-only | `1770` の子 `SPEC` として plan/tasks を追加し、未実装なら正式に着手管理する |
| `SPEC-1781` | AI ブランチ命名 | open / spec-only | wizard 実装を正本に plan/tasks を backfill し、`1644` との境界を明記する |
| `SPEC-1782` | Quick Start — ブランチ単位のワンクリック起動 | open / 完了間近 | タイトルと `id` を正規化し、最終 task 完了後に close する |
| `SPEC-1783` | ヘルプオーバーレイ | open / spec-only | `1770` / `1776` の子 `SPEC` として plan/tasks を追加する |
| `SPEC-1785` | SPECs 画面からの Agent 起動 | open / 子SPEC | `1782` との差分を「SPEC 起点導線」に限定し、親子関係を明記する |

### 4. GitHub / Project / App Surface

| SPEC | 推奨タイトル | 判定 | 次アクション |
|---|---|---|---|
| `SPEC-1354` | Issue タブ — アーティファクト正本 SPEC 詳細互換 | open / sibling | `1643` / `1714` との境界は維持しつつ、タイトルと見出しを正規化する |
| `SPEC-1642` | Docker 実行ターゲットとサービス検出 | open / `TBD` plan / scope過大 | lifecycle/監視/ネットワーク管理を削り、`1552` と整合する UI 導線 `SPEC` に縮小する |
| `SPEC-1643` | GitHub 連携（探索・検索・バージョン履歴） | open / canonical | `1354` / `1714` / `1775` の sibling 関係を本文先頭で固定する |
| `SPEC-1645` | 設定画面と設定カテゴリ構成 | open / `TBD` plan / scope過大 | shortcut/voice/docker/custom-agent は子 `SPEC` に委譲し、Settings 画面構成に限定する |
| `SPEC-1647` | プロジェクト管理（旧仕様） | closed / superseded | `1787` に superseded であることを先頭に明記し、履歴 `SPEC` として凍結する |
| `SPEC-1650` | プロジェクトファイルインデックス | open / tasks 完了 | close 候補。`1784` からは files search 正本として参照する |
| `SPEC-1651` | 通知とエラーバス | open / `TBD` plan / outdated | Tauri/OS 通知前提を除去し、TUI notification/log bus に再定義する |
| `SPEC-1652` | ビルドと配布 | open / `TBD` plan / outdated | Tauri build 記述を削除し、`gwt-tui` の release pipeline に全面更新する |
| `SPEC-1656` | プロファイル設定 TOML 後方互換性 | open / tasks 完了 | close 候補。関連する設定正本への参照だけ残す |
| `SPEC-1714` | Worktree と Issue のリンク・ローカルキャッシュ | open / tasks 完了 | close 候補。`1643` / `1354` から参照のみ残す |
| `SPEC-1775` | gwt-pr-check 統合ステータスレポート | open / spec-only | `1643` の子 `SPEC` として plan/tasks を追加し、スキル実装との差分を埋める |
| `SPEC-1784` | SPEC セマンティック検索と検索命名規約 | open / spec-only | `1643` + `1579` 連携の子 `SPEC` として plan/tasks を追加する |

## 優先是正順序

### P0: 先に片付けるべき構造問題

- `metadata.id` の不統一 10 件を数値文字列へ揃える
  - `SPEC-1777` 〜 `SPEC-1786` のうち `metadata.id = SPEC-*` 形式のもの
- `open + spec-only` 8 件を処理する
  - `SPEC-1775`, `1777`, `1778`, `1779`, `1780`, `1781`, `1783`, `1784`
- `open + TBD plan` 6 件を処理する
  - `SPEC-1642`, `1645`, `1646`, `1648`, `1651`, `1652`
- canonical 衝突を本文先頭で明記する
  - `1541/1770/1776`
  - `1354/1643/1714/1775`
  - `1579/1787`
  - `1552/1642`
  - `1636/1646/1779`

### P1: 規約統一

- 全 `SPEC` を日本語タイトルへ正規化する
- `Feature Specification:` / `機能仕様:` / `gwt-spec:` 接頭辞を外す
- `spec.md` の見出しを共通テンプレートへ寄せる
- `closed` の legacy `SPEC` には `superseded` / `historical` 注記を追加する

### P2: 状態の整合化

- open なのに tasks 完了の `SPEC` を close 判定する
  - `SPEC-1636`, `1644`, `1650`, `1656`, `1714`
- closed なのに tasks 未完了の `SPEC` を履歴扱いへ明示する
  - `SPEC-1438`, `1540`, `1543`, `1544`, `1552`, `1578`, `1647`
- `SPEC-1776` の子 `SPEC` 群は、実装済みのものから close へ寄せる

## 監査結論

今の `SPEC` 群の主問題は「量」ではなく「正本の衝突」と「状態の不整合」です。
優先順位は次の順で進めるべきです。

1. `canonical / child / superseded` を全 `SPEC` に明記する
2. `open` の stub と `TBD plan` を解消する
3. `open-all-done` と `closed-task-mismatch` を是正する
4. 最後にタイトルと見出しを一括で揃える

補足として、`docs/architecture.md` はまだ GUI/Tauri 時代の説明であり、`SPEC-1776` / `SPEC-1654` / README の現状とズレています。SPEC 正規化の後段で更新対象に含めるべきです。
