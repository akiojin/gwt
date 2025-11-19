# 機能仕様: Qwen CLIビルトインツール統合

**仕様ID**: `SPEC-afd20ca6`
**作成日**: 2025-11-19
**ステータス**: ドラフト
**入力**: ユーザー説明: "Qwen CLIビルトインツール統合"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - AIツール選択とQwen起動 (優先度: P1)

開発者がgwtを起動し、AIツール選択画面でQwenを選択して、選択したブランチのworktree内でQwen CLIセッションを開始する。

**この優先度の理由**: これは機能の中核であり、Qwenをビルトインツールとして使用するための基本的な流れ。この機能単独でQwen CLIの基本的な利用価値を提供する。

**独立したテスト**: AIツール選択画面でQwenを選択し、起動することで完全にテスト可能。ユーザーは既存のClaude CodeやCodex、Geminiと同様にQwenを利用できる価値を得る。

**受け入れシナリオ**:

1. **前提条件** gwtが起動され、ブランチが選択されている、**操作** AIツール選択画面でQwenを選択、**期待結果** Qwen CLIが起動し、worktreeディレクトリで対話セッションが開始される
2. **前提条件** ローカルにqwenコマンドがインストールされている、**操作** Qwenを選択して起動、**期待結果** ローカルのqwenコマンドが使用され、起動メッセージに「Using locally installed qwen command」と表示される
3. **前提条件** ローカルにqwenコマンドが存在しない、**操作** Qwenを選択して起動、**期待結果** bunx経由でQwen CLIが起動し、「Falling back to bunx @qwen-code/qwen-code@latest」と表示される

---

### ユーザーストーリー 2 - セッション管理機能の利用 (優先度: P2)

開発者がQwen CLI内で作業セッションを保存し、後で同じ状態から再開できるようにする。

**この優先度の理由**: セッション管理により作業の継続性が向上し、開発者の生産性が向上する。P1の基本機能がなくても価値はあるが、P1が動作している前提で最大の価値を発揮する。

**独立したテスト**: Qwen CLI起動後に `/chat save test-session` を実行し、終了後に再起動して `/chat resume test-session` を実行することでテスト可能。セッション状態が復元されることを確認できる。

**受け入れシナリオ**:

1. **前提条件** Qwen CLIが--checkpointingフラグ付きで起動されている、**操作** 対話中に `/chat save mysession` を実行、**期待結果** セッションが保存され、確認メッセージが表示される
2. **前提条件** 保存済みセッション「mysession」が存在する、**操作** Qwen CLI起動後に `/chat resume mysession` を実行、**期待結果** 以前の会話履歴が復元され、作業を継続できる
3. **前提条件** Qwen CLIが起動している、**操作** `/chat list` を実行、**期待結果** 保存済みセッションのリストが表示される

---

### ユーザーストーリー 3 - 権限スキップモードでの起動 (優先度: P3)

開発者がgwtで権限スキップモードを有効にした場合、Qwen CLIも自動承認モード（--yolo）で起動し、すべてのアクションを自動で承認する。

**この優先度の理由**: 自動化やCI/CD環境での利用に有用だが、通常の対話的な開発では必須ではない。P1とP2が動作していれば、この機能がなくても基本的な価値は提供される。

**独立したテスト**: gwtで権限スキップモードを有効化し、Qwen CLIを起動することでテスト可能。起動ログに「Auto-approving all actions (YOLO mode)」が表示され、Qwen CLIが確認なしで動作することを確認できる。

**受け入れシナリオ**:

1. **前提条件** gwtで権限スキップモードが有効化されている、**操作** Qwenを選択して起動、**期待結果** Qwen CLIが--yoloフラグ付きで起動し、自動承認モードのメッセージが表示される
2. **前提条件** 権限スキップモードが無効、**操作** Qwenを選択して起動、**期待結果** Qwen CLIが通常モードで起動し、--yoloフラグは付与されない

---

### エッジケース

- qwenコマンドもbunxも利用できない場合、何が起こりますか？
  - エラーメッセージ「bunx command not found. Please ensure Bun is installed so Qwen CLI can run via bunx.」が表示され、QwenErrorがスローされる
- worktreeパスが存在しない場合、システムはどのように処理しますか？
  - エラーメッセージ「Worktree path does not exist: [path]」が表示され、起動が中断される
- Windows環境での起動時にエラーが発生した場合、どのようなトラブルシューティング情報が提供されますか？
  - Windows固有のトラブルシューティングメッセージ（PATHの確認、qwen --versionの実行、ターミナル再起動の推奨）が表示される
- 環境変数に同じキーが共有envとツール固有envの両方に存在する場合、どちらが優先されますか？
  - ツール固有の環境変数が優先される（`{...process.env, ...sharedEnv, ...toolEnv}` の順序）

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムはQwen CLIをビルトインAIツールとして定義し、ID「qwen-cli」、表示名「Qwen」でツール選択画面に表示**しなければならない**
- **FR-002**: システムはQwen CLI起動時にデフォルト引数として「--checkpointing」を含め、セッション管理機能を有効化**しなければならない**
- **FR-003**: システムはローカルにインストールされた「qwen」コマンドを優先的に使用し、見つからない場合は「bunx @qwen-code/qwen-code@latest」にフォールバック**しなければならない**
- **FR-004**: システムはQwen CLI起動時のエラーをQwenErrorクラスでラップし、isRecoverableErrorで回復可能エラーとして扱う**しなければならない**
- **FR-005**: システムは共有環境変数（tools.jsonのenv）とツール固有の環境変数をマージし、ツール固有の変数を優先**しなければならない**
- **FR-006**: ユーザーが権限スキップモードを有効化した場合、システムはQwen CLI起動時に「--yolo」フラグを付与**しなければならない**
- **FR-007**: システムはnormal、continue、resumeのすべてのモードで同じ引数（空配列）を使用**しなければならない**（Qwen CLIには起動時の明示的な継続・再開オプションが存在しないため）
- **FR-008**: システムはQwen CLI起動前にworktreeパスの存在を確認し、存在しない場合はエラーを返す**しなければならない**
- **FR-009**: システムはWindows環境でのエラー発生時に、プラットフォーム固有のトラブルシューティングメッセージを表示**しなければならない**
- **FR-010**: システムは起動時にターミナルのRawモードを適切に管理し、終了時にRawモードを解除**しなければならない**

### 主要エンティティ

- **QWEN_CLI_TOOL**: Qwen CLIのビルトインツール定義。id（文字列）、displayName（文字列）、type（"bunx"）、command（文字列）、defaultArgs（文字列配列）、modeArgs（normal/continue/resumeの各モード用の文字列配列）、permissionSkipArgs（文字列配列）を含む
- **QwenError**: Qwen CLI起動時のエラーを表すErrorクラス。message（文字列）、cause（unknown）、name（"QwenError"）を含む
- **LaunchOptions**: Qwen CLI起動時のオプション。skipPermissions（真偽値）、mode（"normal" | "continue" | "resume"）、extraArgs（文字列配列、任意）、envOverrides（キー・値のマップ、任意）を含む

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: 開発者はAIツール選択画面でQwenを他のツール（Claude Code、Codex、Gemini）と同様に選択できる
- **SC-002**: Qwen CLIの起動時間はClaude Code、Codex、Geminiの起動時間と同等（±2秒以内）である
- **SC-003**: Qwen CLI起動エラーの90%以上で、ユーザーが問題を自己解決できるトラブルシューティング情報が提供される
- **SC-004**: セッション管理機能（/chat save、/chat resume）が100%の確率で利用可能である（--checkpointingフラグが常に付与される）
- **SC-005**: 開発者の95%以上が、既存のビルトインツールと同じ操作性でQwenを利用できる

## 制約と仮定 *(該当する場合)*

### 制約

- 既存のビルトインツール機構（src/config/builtin-tools.ts、src/index.ts）を使用する必要がある
- bunまたはbunxが環境にインストールされている必要がある
- Qwen CLIパッケージ（@qwen-code/qwen-code）がnpmレジストリで利用可能である必要がある

### 仮定

- ユーザーはQwen CLIの基本的な使い方（/chatコマンドなど）を理解している
- --checkpointingフラグによりセッション管理機能が有効化される（Qwen CLI公式の仕様に基づく）
- Qwen CLIの--yoloフラグが自動承認モードを有効化する（公式ヘルプに記載）
- 既存のビルトインツール（Claude Code、Gemini）の実装パターンが正しく、それに従うことで一貫性のあるUXが実現される

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- Qwen CLI本体の機能拡張やバグ修正
- カスタムツールとしてのQwen追加（ビルトインツールのみ対象）
- 他のビルトインツール（Claude Code、Codex、Gemini）への機能追加や変更
- Qwen CLI以外のAIツールの統合
- Qwen CLIの設定ファイル（.qwen/settings.json）の自動生成や管理
- モバイルアプリケーションやブラウザ拡張機能

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- 環境変数に機密情報（APIキー、アクセストークンなど）が含まれる可能性があるため、環境変数のマージ順序を適切に管理する必要がある（ツール固有の環境変数が共有環境変数より優先されることで、ユーザーが意図的に設定した値が保持される）
- 権限スキップモード（--yolo）は、すべてのQwen CLIアクションを自動承認するため、本番環境や重要なデータを扱う環境では慎重に使用する必要がある
- Qwen CLIが実行するコマンドやファイル操作は、gwtの権限スキップ設定に依存するため、ユーザーに権限スキップモードのリスクを理解させる必要がある

## 依存関係 *(該当する場合)*

- Qwen CLI (@qwen-code/qwen-code@latest): AIツール本体
- bun/bunx: Qwen CLI実行環境
- execa: プロセス実行ライブラリ
- chalk: ターミナル出力の色付け
- 既存のビルトインツール機構（BUILTIN_TOOLS配列、CustomAITool型定義）
- 既存のエラーハンドリング機構（isRecoverableError関数）
- 既存のターミナル管理機構（getTerminalStreams、createChildStdio）

## 参考資料 *(該当する場合)*

- [Qwen Code公式ドキュメント - CLI Commands](https://qwenlm.github.io/qwen-code-docs/en/cli/commands/)
- [Qwen Code公式ドキュメント - Configuration](https://qwenlm.github.io/qwen-code-docs/en/cli/configuration/)
- [既存実装: Claude Code (src/claude.ts)](../../../src/claude.ts)
- [既存実装: Gemini CLI (src/gemini.ts)](../../../src/gemini.ts)
- [既存実装: ビルトインツール定義 (src/config/builtin-tools.ts)](../../../src/config/builtin-tools.ts)
