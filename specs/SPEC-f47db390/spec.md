# 機能仕様: セッションID永続化とContinue/Resume強化

**仕様ID**: `SPEC-f47db390`  
**作成日**: 2025-12-06  
**ステータス**: ドラフト  
**入力**: ユーザー説明: "Continueオプションで実際に使ったセッションを確実に再開できるよう、セッションIDを保存して起動時に提示したい（Codexは終了時に resume コマンドを表示、Claude Codeは /status でしか見られないので不便）"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Continue/Resumeが必ず正しいセッションに接続できる (優先度: P1)

開発者が「Continue」または「Resume」でgwtを起動したとき、直前に使っていたCodex/Claude CodeのセッションIDを自動で指定し、誤った会話を開かない。

**この優先度の理由**: 誤セッション再開は作業ロスが大きく、最も頻繁に使う「続きから」動作の信頼性を保証するため。  
**独立したテスト**: 新規セッションを開始→終了→セッションIDを保存→再度「Continue」を選択。CLIに`codex resume <ID>`または`claude --resume <ID>`が渡され、履歴が継続することを確認。

**受け入れシナリオ**:
1. **前提条件** gwtでCodexを新規起動済み、**操作** 退出後に「Continue」を選択、**期待結果** Codexが保存済みIDで`codex resume <ID>`として起動する。
2. **前提条件** gwtでClaude Codeを新規起動済み、**操作** 退出後に「Continue」を選択、**期待結果** Claude Codeが`claude --resume <ID>`で起動し、同じ会話が開く。
3. **前提条件** 保存済みIDが欠落/期限切れ、**操作** 「Continue」を選択、**期待結果** 旧挙動（Codex: `--last`、Claude: `-c`）にフォールバックし、警告を表示。

---

### ユーザーストーリー 2 - セッションIDを退出時に即確認できる (優先度: P1)

開発者がセッション終了時に、再開に必要なセッションIDと具体的な再開コマンドを画面で確認できる。

**この優先度の理由**: CLI内でコマンド入力ができない環境でも再開方法を控えられるため、可用性が向上する。  
**独立したテスト**: Codex/Claude Code終了直後の出力に「Session ID」と「再開コマンド例」が表示されることを確認（ログキャプチャで検証可能）。

**受け入れシナリオ**:
1. **前提条件** Codexセッション終了直後、**操作** 標準出力を確認、**期待結果** `Session ID: <uuid>` と `Resume: codex resume <uuid>` が表示される。
2. **前提条件** Claude Code終了直後、**操作** 標準出力を確認、**期待結果** `Session ID: <uuid>` と `Resume: claude --resume <uuid>` が表示される。

---

### ユーザーストーリー 3 - 手動選択用のセッション一覧が参照できる (優先度: P2)

開発者が「Resume」モードを選ぶと、保存済みセッションの一覧（ID/ツール/ブランチ/開始時刻）が表示され、任意のセッションを指定して再開できる。

**この優先度の理由**: 複数作業を並行するケースで「どのセッションか」を明示的に選べることが利便性につながる。  
**独立したテスト**: セッション履歴を2件以上保存→ResumeモードでSessionSelectorに一覧が表示され、選択したIDでCLIが起動することを確認。

**受け入れシナリオ**:
1. **前提条件** 複数のCodex/Claudeセッションが保存済み、**操作** Resumeモードで1件選択、**期待結果** 選択IDで`codex resume`または`claude --resume`が実行される。
2. **前提条件** 一覧に有効IDがない、**操作** Resumeモード、**期待結果** 「保存されたセッションがありません」警告を出し通常起動に戻す。

---

### ユーザーストーリー 4 - Gemini/Qwenでも同等の再開体験 (優先度: P2)

開発者がGeminiまたはQwenを利用するときも、セッションID（または保存タグ）を保存・表示し、Continue/Resumeで最新セッションを再開できる。

**この優先度の理由**: マルチツール利用時の体験差異をなくし、一貫した「続きから」操作を提供するため。  
**独立したテスト**: Gemini/Qwenで1セッション実行→終了→Continue/Resume起動時にID/タグが表示・渡されることを確認。

**受け入れシナリオ**:
1. **前提条件** Geminiセッション実行済み、**操作** gwtでContinue、**期待結果** `gemini --resume <ID>` が渡され同じ会話が開く（IDがない場合は最新にフォールバック）。
2. **前提条件** Qwenセッションを `/chat save foo` で保存済み、**操作** gwtでContinue、**期待結果** 保存タグが表示され、ログに `/chat resume foo` を実行する案内が出る（自動入力不可の場合は手動案内）。
3. **前提条件** Gemini/Qwenで履歴無し、**操作** Continue、**期待結果** 従来の新規起動にフォールバックし警告を表示。

---

### エッジケース
- セッションディレクトリ（`~/.codex/sessions` や `~/.claude/projects/.../sessions`）が存在しない/権限不足。
- 24時間ルールで保存済みセッションが期限切れの場合のフォールバック動作。
- Windows/WSLパス差異でセッションファイル探索に失敗する場合。
- 非対応ツール（Gemini/Qwen/カスタム）の場合は従来挙動を維持する。

## 要件 *(必須)*

### 機能要件
- **FR-001**: gwtはAIツール起動終了時にツールのセッションIDを取得し、リポジトリ単位の`SessionData`に `lastSessionId` と履歴エントリの`sessionId`を保存しなければならない（後方互換のため任意フィールドとして扱う）。
- **FR-002**: CodexのセッションID取得は終了後に`~/.codex/sessions/*.json`またはCLI出力を走査し、最新セッションIDを特定して保存しなければならない。
- **FR-003**: Claude CodeのセッションID取得は終了後に`~/.claude/projects/<encoded cwd>/sessions/*.jsonl`の最新ファイルを読み取り、メタデータのIDを保存しなければならない。
- **FR-004**: 「Continue」実行時、保存済みIDが存在すればCodexには`codex resume <id>`、Claude Codeには`claude --resume <id>`を渡し、存在しない場合は従来の`--last`/`-c`にフォールバックしなければならない。
- **FR-005**: 「Resume」実行時、保存済み履歴（最大直近100件）を一覧表示し、選択されたエントリの`sessionId`とツール種別に応じた再開コマンドで起動しなければならない。
- **FR-006**: セッション終了時に「Session ID」「Resumeコマンド例」「保存先パス」をユーザーに表示し、必要ならコピーできるようにしなければならない。
- **FR-007**: セッション保存・読み出しが失敗してもワークフローをブロックしないこと。失敗時は警告を表示し、デフォルト起動に戻る。
- **FR-008**: Gemini/Qwen/カスタムツールなどセッションIDを提供しないツールでは、既存の保存ロジックを変更せず、Continue/ResumeでIDを要求しない。
- **FR-009**: Gemini CLIでは終了後に`~/.gemini/tmp/<project_hash>/chats/*.json`の最新ファイルからIDを抽出し、Continue/Resume時は`--resume <id>`を優先、ID不明時は`--resume`（latest）にフォールバックしなければならない。
- **FR-010**: Qwen CLIでは終了後に`~/.qwen/tmp/<project_hash>/`配下の保存ファイル（/chat save or checkpoint）からタグ/IDを抽出し履歴に保存しなければならない。Continue/Resume時には保存タグを表示し、`/chat resume <tag>` の案内を必ず出すこと（自動再開できない場合のフォールバック）。

### 主要エンティティ
- **SessionData**: `lastWorktreePath`, `lastBranch`, `lastUsedTool`, `mode`, `model`, 追加で `lastSessionId` を持つ。履歴`history[]`に`sessionId`/`toolId`/`branch`/`timestamp`を保持。
- **ToolSessionEntry**: `sessionId`, `toolId`, `toolLabel`, `branch`, `worktreePath`, `mode`, `model`, `timestamp`.

## 成功基準 *(必須)*

### 測定可能な成果
- **SC-001**: Codex/Claude Codeの正常終了後、90%以上のケースで`SessionData`に`sessionId`が保存される（ローカルログで確認）。
- **SC-002**: Continue実行時に保存済みIDが存在する場合、100%のケースでCLI引数に該当IDが渡される。
- **SC-003**: セッション終了メッセージで再開コマンドが表示されることを手動確認できる（回帰テストスクリプトでstdoutを検査）。
- **SC-004**: 非対応ツール選択時に従来機能（新規起動）が阻害されないことを自動テストで確認。

## 制約と仮定 *(該当する場合)*

### 制約
- Codexのセッションファイルは`~/.codex/sessions/`配下に書き出される想定で、読み取り専用で扱う。
- Claude Codeのセッションはカレントディレクトリに基づき`~/.claude/projects/<path-encoded>/sessions/`に保存されるため、パスエンコードロジックを実装する必要がある。
- 24時間以上前のセッションは既存ポリシー通り無効扱いとする。

### 仮定
- Codex/Claude CodeはセッションIDをJSON/JSONLに保存しており、ファイルの最新更新時刻で直近セッションを特定できる。
- CLIオプションは最新リファレンスに従い、Codexは`resume <SESSION_ID>`、Claude Codeは`--resume <session-id>`で再開できる。

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- Web UIでのセッション一覧表示やコピー操作
- セッションIDのクラウド同期・共有機能
- 他ツール(Gemini/Qwen/カスタム)のセッション管理実装
- Claude/Codex本体の挙動変更やセッション保持期間の延長

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- セッションIDは機密情報ではないが作業履歴に紐づくため、保存先はユーザーのホームディレクトリ配下に限定し、外部送信しない。
- 取得に失敗した場合でもスタックトレースを標準出力に流さず、DEBUGフラグ時のみ詳細ログを出す。

## 依存関係 *(該当する場合)*

- Codex CLIの再開コマンド（`codex resume <SESSION_ID>`）とセッションストレージ`~/.codex/sessions`。
- Claude Code CLIの再開コマンド（`claude --resume <session-id>`）とプロジェクト別ストレージ`~/.claude/projects/<encoded>/sessions/`。
- 既存のセッション保存ロジック（`src/config/index.ts`）とUIフロー（ExecutionModeSelector/SessionSelector）。

## 参考資料 *(該当する場合)*

- [Codex CLI リファレンス: resume サブコマンド](https://developers.openai.com/codex/cli/reference/)
- [Claude Code CLI リファレンス: --resume / --continue フラグ](https://docs.claude.com/en/docs/claude-code/cli-usage)
- [Claude Code セッション保存場所例 (`~/.claude/projects/.../sessions/*.jsonl`)](https://www.reddit.com/r/ClaudeAI/comments/1pa0s0h/is_there_a_way_to_have_claude_code_search_the/)
- [Codex セッションファイルが `~/.codex/sessions` に保存される事例](https://github.com/openai/codex/issues/3817)
