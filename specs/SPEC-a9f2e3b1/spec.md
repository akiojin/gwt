# 機能仕様: mergeStateStatus UNKNOWN リトライ

**仕様ID**: `SPEC-a9f2e3b1`
**作成日**: 2026-02-26
**更新日**: 2026-02-26
**ステータス**: レビュー中
**カテゴリ**: GUI / Backend
**依存仕様**:

- SPEC-d6949f99（PRステータス取得）
- SPEC-merge-pr（マージ機能）

**入力**: ユーザー説明: "gh pr view の mergeStateStatus が一時的に UNKNOWN を返す場合、Worktree詳細ビューで Unknown のまま表示される。リトライして解決すべき"

## 背景

- GitHub GraphQL API は PR の `mergeStateStatus` および `mergeable` フィールドで一時的に `UNKNOWN` を返すことがある
- 典型的にはマージブランチの更新チェック中（5-10秒）だが、リベースや大規模リポジトリではそれ以上かかる
- PR 作成直後も永続的に `UNKNOWN` になるケースがある
- 現在の実装ではリトライロジックがなく、`UNKNOWN` がそのまま UI に表示される
- これにより Worktree 詳細ビューのマージ可能性バッジが "Unknown" 表示、マージボタンが無効化されたままになる

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - UNKNOWN の自動リトライ (優先度: P0)

ユーザーとして、PRの mergeStateStatus が一時的に UNKNOWN でも、バックグラウンドで自動リトライされ、最終的に正しいステータスが表示されてほしい。

**独立したテスト**: UNKNOWN レスポンスを受信後、バックグラウンドリトライが起動し、解決済みステータスでキャッシュが更新される

**受け入れシナリオ**:

1. **前提条件** PR が存在し GraphQL API が mergeStateStatus=UNKNOWN を返す、**操作** fetch_pr_status が呼ばれる、**期待結果** レスポンスに retrying=true フラグが付き、バックグラウンドでリトライタスクが起動する
2. **前提条件** リトライ中の PR がある、**操作** バックグラウンドリトライが CLEAN を取得する、**期待結果** キャッシュが即座に更新され、Tauri イベントでフロントエンドに通知される
3. **前提条件** 5回リトライしても UNKNOWN のまま、**操作** リトライ上限到達、**期待結果** リトライを停止し通常ポーリングサイクルに復帰。次回ポーリングで再度 UNKNOWN なら新たなリトライサイクルを開始

---

### ユーザーストーリー 2 - リトライ中の UI 表示 (優先度: P0)

ユーザーとして、PR ステータスが確認中であることが視覚的に分かるようにしてほしい。

**独立したテスト**: retrying=true のとき、PrStatusSection のバッジがパルスアニメーション、マージボタンが disabled

**受け入れシナリオ**:

1. **前提条件** retrying=true の PR がある、**操作** PrStatusSection を表示、**期待結果** mergeable バッジがパルス（点滅）アニメーションで表示され、マージボタンは disabled + "Checking merge status..." テキスト
2. **前提条件** retrying=true の PR がある、**操作** サイドバーの Worktree リストを表示、**期待結果** 対象ブランチの PR ステータス表示もパルスアニメーション
3. **前提条件** リトライが解決し retrying=false になった、**操作** Tauri イベント受信、**期待結果** パルスアニメーションが停止し、正しいステータスバッジが表示される

---

### ユーザーストーリー 3 - キャッシュ退行防止 (優先度: P1)

ユーザーとして、一度確認できた正常な mergeable / mergeStateStatus が UNKNOWN に退行しないでほしい。

**独立したテスト**: キャッシュに MERGEABLE が格納済みの PR に対し、新しい取得結果が UNKNOWN の場合、キャッシュの該当フィールドが上書きされない

**受け入れシナリオ**:

1. **前提条件** キャッシュに mergeable=MERGEABLE, mergeStateStatus=CLEAN の PR がある、**操作** 新しい取得結果で mergeable=UNKNOWN が返る、**期待結果** キャッシュの mergeable, mergeStateStatus フィールドは既存値を維持し上書きされない
2. **前提条件** キャッシュが空（初回取得）、**操作** mergeable=UNKNOWN が返る、**期待結果** UNKNOWN の値がキャッシュに格納される（初回は退行ではないため）

## エッジケース

- リトライ中にPRがクローズ/マージされた場合: 新しい状態をそのまま反映（UNKNOWN チェックはスキップ）
- 同一PRに対する複数リトライタスクの重複起動: PrStatusCache の retrying フラグで排他制御
- レート制限中のリトライ: cooldown_until が設定されていればリトライをスキップし、通常ポーリングに任せる
- ページがバックグラウンド中のリトライ完了: キャッシュ更新 + イベント発行は行うが、フロントエンドのリスナーが非活性でも復帰時のポーリングでキャッシュから正常値を取得
- PR detail (fetch_pr_detail) でも UNKNOWN が返った場合: detail はキャッシュ構造が異なるため、本仕様のスコープ外（ただし将来対応の検討余地あり）

## 要件 *(必須)*

### 機能要件

- **FR-001**: `fetch_pr_status_impl` で取得結果に `mergeable=UNKNOWN` または `mergeStateStatus=UNKNOWN` の PR がある場合、レスポンスの該当 PR に `retrying=true` を付与し、バックグラウンドリトライタスクを起動する
- **FR-002**: バックグラウンドリトライは指数バックオフ（2s, 4s, 8s, 16s, 32s）で最大5回実行する
- **FR-003**: リトライには既存の `build_pr_status_query` を UNKNOWN PR のブランチ名のみで再利用する
- **FR-004**: リトライで UNKNOWN が解決した場合、キャッシュを即座に更新し、Tauri イベント `pr-status-updated` をフロントエンドに emit する
- **FR-005**: 既存キャッシュに正常値がある PR について、新しい取得結果の `mergeable` または `mergeStateStatus` が UNKNOWN の場合、該当フィールドのキャッシュ上書きをスキップする
- **FR-006**: `PrStatusLiteSummary` および `PrStatusResponse` に `retrying` フラグを追加し、フロントエンドに伝達する
- **FR-007**: `PrStatusCache`（`RepoPrStatusCacheEntry`）に PR 単位のリトライ状態（retrying フラグ、retry_count）を追加する
- **FR-008**: 同一 PR に対するリトライタスクの重複起動を防止する
- **FR-009**: フロントエンドは `retrying=true` の場合、PrStatusSection のバッジおよびサイドバーの PR ステータス表示にパルスアニメーションを適用する
- **FR-010**: リトライ中のマージボタンは disabled にし、"Checking merge status..." を表示する
- **FR-011**: 5回リトライ後も未解決の場合、通常ポーリング（30秒）に復帰する。次回ポーリングで再度 UNKNOWN ならリトライサイクルを再開する
- **FR-012**: マージボタンは retrying 中および UNKNOWN 未解決時は disabled を維持する

### 非機能要件

- **NFR-001**: リトライによる GitHub API 呼び出し増加は最大5回/PR/サイクル。レート制限 cooldown 中はリトライをスキップする
- **NFR-002**: バックグラウンドリトライは Tauri コマンド応答をブロックしない（即座返却方式）
- **NFR-003**: パルスアニメーションは CSS のみで実現し、JavaScript タイマーを使用しない

## 制約と仮定

- GitHub GraphQL API の UNKNOWN は通常5-30秒で解決するが、永続的な場合もある
- 既存の `build_pr_status_query` は単一ブランチでも動作する（ブランチ名配列で1要素を渡す）
- Tauri の `emit` はフロントエンドのウィンドウがバックグラウンドでもバッファされる

## 成功基準 *(必須)*

- **SC-001**: mergeStateStatus が一時的に UNKNOWN を返す PR について、62秒以内（5回リトライ完了）に正しいステータスが表示される（API が正常に解決した場合）
- **SC-002**: UNKNOWN 状態の PR に対してユーザーに "Unknown" のまま放置される状況が発生しない（リトライ中はパルスアニメーションで確認中であることが伝わる）
- **SC-003**: 既に正常値がキャッシュされている PR が UNKNOWN に退行しない
- **SC-004**: リトライによる API 呼び出しがレート制限に抵触しない
