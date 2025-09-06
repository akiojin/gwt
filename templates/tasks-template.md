# タスク: [FEATURE NAME]

**入力**: `/specs/[###-feature-name]/` の設計ドキュメント
**前提**: plan.md（必須）、research.md、data-model.md、contracts/

## 実行フロー（main）
```
1. 機能ディレクトリの plan.md を読み込む
   → ない場合: ERROR "実装計画が見つかりません"
   → 抽出: 技術スタック、ライブラリ、構成
2. 任意の設計ドキュメントを読み込む:
   → data-model.md: エンティティを抽出 → モデルタスク
   → contracts/: 各ファイル → コントラクトテストタスク
   → research.md: 決定事項を抽出 → セットアップタスク
3. カテゴリ別にタスクを生成:
   → Setup: プロジェクト初期化、依存、Lint 設定
   → Tests: コントラクトテスト、統合テスト
   → Core: モデル、サービス、CLI コマンド
   → Integration: DB、ミドルウェア、ログ
   → Polish: ユニットテスト、性能、ドキュメント
4. タスクリールを適用:
   → 異なるファイル = 並列可として [P]
   → 同一ファイル = 逐次（[P] なし）
   → 実装前にテスト（TDD）
5. タスクに連番を付与（T001, T002...）
6. 依存グラフを生成
7. 並列実行例を作成
8. タスクの完全性を検証:
   → すべての契約にテストがあるか？
   → すべてのエンティティにモデルがあるか？
   → すべてのエンドポイントが実装されるか？
9. 戻り値: SUCCESS（実行可能なタスクが整備）
```

## 形式: `[ID] [P?] 説明`
- **[P]**: 並列実行可能（異なるファイル・依存なし）
- 説明には正確なファイルパスを含める

## パス規約
- **単一プロジェクト**: リポジトリ直下に `src/`、`tests/`
- **Web アプリ**: `backend/src/`、`frontend/src/`
- **モバイル**: `api/src/`、`ios/src/` または `android/src/`
- 以下のパスは単一プロジェクトを前提。plan.md の構成に合わせて調整すること

## フェーズ 3.1: セットアップ
- [ ] T001 実装計画に従いプロジェクト構成を作成
- [ ] T002 [language] プロジェクトを [framework] 依存付きで初期化
- [ ] T003 [P] Lint とフォーマッタの設定

## フェーズ 3.2: まずテスト（TDD） ⚠️ 3.3 の前に必須
**重要: これらのテストは実装前に必ず作成し、必ず失敗していなければならない**
- [ ] T004 [P] コントラクトテスト POST /api/users （tests/contract/test_users_post.py）
- [ ] T005 [P] コントラクトテスト GET /api/users/{id} （tests/contract/test_users_get.py）
- [ ] T006 [P] 統合テスト ユーザー登録 （tests/integration/test_registration.py）
- [ ] T007 [P] 統合テスト 認証フロー （tests/integration/test_auth.py）

## フェーズ 3.3: コア実装（テストが失敗状態になってからのみ）
- [ ] T008 [P] ユーザーモデル（src/models/user.py）
- [ ] T009 [P] UserService の CRUD（src/services/user_service.py）
- [ ] T010 [P] CLI --create-user（src/cli/user_commands.py）
- [ ] T011 POST /api/users エンドポイント
- [ ] T012 GET /api/users/{id} エンドポイント
- [ ] T013 入力バリデーション
- [ ] T014 エラーハンドリングとログ

## フェーズ 3.4: 連携
- [ ] T015 UserService を DB に接続
- [ ] T016 認証ミドルウェア
- [ ] T017 リクエスト/レスポンスのロギング
- [ ] T018 CORS とセキュリティヘッダー

## フェーズ 3.5: 仕上げ
- [ ] T019 [P] バリデーションのユニットテスト（tests/unit/test_validation.py）
- [ ] T020 性能テスト（<200ms）
- [ ] T021 [P] ドキュメント更新（docs/api.md）
- [ ] T022 重複の排除
- [ ] T023 manual-testing.md の実施

## 依存関係
- 実装（T008-T014）より先にテスト（T004-T007）
- T008 が T009, T015 をブロック
- T016 が T018 をブロック
- 仕上げ（T019-T023）は実装の後

## 並列実行例
```
# T004〜T007 を同時に走らせる:
Task: "Contract test POST /api/users in tests/contract/test_users_post.py"
Task: "Contract test GET /api/users/{id} in tests/contract/test_users_get.py"
Task: "Integration test registration in tests/integration/test_registration.py"
Task: "Integration test auth in tests/integration/test_auth.py"
```

## 注意事項
- [P] タスク = 異なるファイルで依存なし
- 実装前にテストが失敗していることを確認
- 各タスクごとにコミット
- 回避事項: 曖昧なタスク、同一ファイルでの競合

## タスク生成ルール
*main() 実行中に適用*

1. **契約から**:
   - 契約ファイルごとに → コントラクトテストタスク [P]
   - エンドポイントごとに → 実装タスク
   
2. **データモデルから**:
   - エンティティごとに → モデル作成タスク [P]
   - 関係性 → サービス層タスク
   
3. **ユーザーストーリーから**:
   - ストーリーごとに → 統合テスト [P]
   - クイックスタートシナリオ → 検証タスク

4. **順序**:
   - Setup → Tests → Models → Services → Endpoints → Polish
   - 依存関係は並列実行をブロック

## 検証チェックリスト
*GATE: 戻る前に main() が確認*

- [ ] すべての契約に対応するテストがある
- [ ] すべてのエンティティにモデルタスクがある
- [ ] すべてのテストが実装より前に来る
- [ ] 並列タスクが真に独立している
- [ ] 各タスクが正確なファイルパスを指定している
- [ ] [P] タスク同士で同一ファイルを変更しない
