# 機能仕様: セッションID永続化とContinue/Resume強化

**仕様ID**: `SPEC-f47db390`  
**作成日**: 2025-12-06  
**ステータス**: 更新中  
**実装フェーズ**: Phase 2（実装完了）+ Web UI追記  
**最終更新**: 2026-01-07  
**次のステップ**: 運用中（必要に応じて改善）  
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

### ユーザーストーリー 3 - ResumeはツールのResume機能を呼び出す (優先度: P2)

開発者が「Resume」モードを選ぶと、gwtは独自のセッション一覧を表示せず、選択したAIツールが提供するResume機能で再開フローを開始する。

**この優先度の理由**: セッションの真正なソースは各ツール側の履歴であり、gwt側の履歴不足でも確実に再開操作に到達できるため。  
**独立したテスト**: Resumeモードを選択→gwtがセッションIDを自動補完しないことを確認しつつ、ツール起動引数がツール固有のResume形式（Codex: `resume`、Claude Code: `-r`、Gemini: `--resume`）になることを検証する。

**受け入れシナリオ**:
1. **前提条件** 任意のAIツール選択済み、**操作** Resumeモードを選択、**期待結果** gwtはSessionSelectorを表示せず、ツール固有のResume起動引数で起動する。
2. **前提条件** gwt側の履歴が空/欠落、**操作** Resumeモードを選択、**期待結果** gwtは履歴に依存せずツールのResumeを起動し、ツール側の標準挙動でセッション選択/最新再開が行える。

---

### ユーザーストーリー 4 - Geminiでも同等の再開体験 (優先度: P2)

開発者がGeminiを利用するときも、セッションIDを保存・表示し、Continue/Resumeで最新セッションを再開できる。

**この優先度の理由**: マルチツール利用時の体験差異をなくし、一貫した「続きから」操作を提供するため。  
**独立したテスト**: Geminiで1セッション実行→終了→Continue/Resume起動時にIDが表示・渡されることを確認。

**受け入れシナリオ**:
1. **前提条件** Geminiセッション実行済み、**操作** gwtでContinue、**期待結果** `gemini --resume <ID>` が渡され同じ会話が開く（IDがない場合は最新にフォールバック）。
2. **前提条件** Geminiで履歴無し、**操作** Continue、**期待結果** 従来の新規起動にフォールバックし警告を表示。

---

### ユーザーストーリー 5 - ブランチ選択直後に前回設定で素早く再開/新規 (優先度: P1)

開発者がブランチを選択したら、前回そのブランチで使ったAIツール・モデル・セッションIDを基に「前回設定で続きから」「前回設定で新規」あるいは「設定を選び直す」を選べる。

**この優先度の理由**: 毎回ツールとモデルを選択する手間を削減し、誤操作なく高速に再開できるようにするため。

**独立したテスト**: ブランチに紐づく履歴を1件残した状態でブランチを選択 → クイック選択画面で「前回設定で続きから」を選ぶと、同じツール/モデルが事前選択され、Continue/Resumeフローに進む。履歴が無い場合は従来のツール選択画面へフォールバックすることを確認。

**受け入れシナリオ**:
1. **前提条件** 対象ブランチの履歴に`toolId/model/sessionId`がある、**操作** ブランチ選択→「前回設定で続きから」を選択、**期待結果** ツール・モデルが前回値でセットされ、Continue/Resumeモード選択へ進み、sessionIdが事前入力される。
2. **前提条件** 対象ブランチの履歴に`toolId/model`はあるが`sessionId`が無い、**操作** ブランチ選択→「前回設定で新規」を選択、**期待結果** ツール・モデルが前回値でセットされ、新規開始モードで起動フローに進む。
3. **前提条件** 対象ブランチに履歴が無い、**操作** ブランチ選択、**期待結果** クイック選択をスキップし従来のツール選択画面に遷移する。
4. **表示ルール** Quick Startではツール固有の情報のみ表示する。CodexではReasoningレベルを表示するが、Claude/GeminiではReasoningを表示しない。また「Start new with previous settings」ではセッションIDを表示しない（IDは「Resume with previous settings」のみで提示する）。
5. **ツール別保持** 同一ブランチ内で複数ツールを切り替えた場合でも、各ツールの直近設定を並列に保持・提示し、Quick Startでツールごとの「Resume/Start new」を選べること（例: Codex行とClaude行が並ぶ）。

---

### ユーザーストーリー 6 - Web UIでも保存済みセッションIDを表示・再開できる (優先度: P1)

開発者がWeb UIのブランチ詳細を開いたとき、直近のセッションIDを確認でき、Continue/Resumeの起動時にそのIDで確実に再開できる。

**この優先度の理由**: Web UIからの起動が増えており、CLI同等の再開信頼性がないと作業ロスが発生するため。  
**独立したテスト**: Web UIでセッション終了→セッションIDが保存される→ブランチ詳細でIDが表示される→Continue/Resumeで`--resume <id>`/`resume <id>`が渡されることを確認。

**受け入れシナリオ**:
1. **前提条件** Web UIでClaude/Codexセッションを実行済み、**操作** ブランチ詳細を開く、**期待結果** 最終ツール使用にセッションIDが表示される。
2. **前提条件** 保存済みセッションIDがあり同一ツールを選択、**操作** Continue/Resumeで起動、**期待結果** そのIDがCLI引数として渡され同じ会話が開く。
3. **前提条件** 保存済みセッションIDが無い、**操作** Continue/Resumeで起動、**期待結果** ツールの標準挙動（Codex: `resume --last` / Claude: `-c` or `-r`）にフォールバックする。

---

### エッジケース
- セッションディレクトリ（`~/.codex/sessions` や `~/.claude/projects/.../sessions`）が存在しない/権限不足。
- 24時間ルールで保存済みセッションが期限切れの場合のフォールバック動作。
- Windows/WSLパス差異でセッションファイル探索に失敗する場合。
- 非対応ツール（カスタムなど）の場合は従来挙動を維持する。

## 要件 *(必須)*

### 機能要件
- **FR-001**: gwtはAIツール起動終了時にツールのセッションIDを取得し、リポジトリ単位の`SessionData`に `lastSessionId` と履歴エントリの`sessionId`を保存しなければならない（後方互換のため任意フィールドとして扱う）。
- **FR-002**: CodexのセッションID取得は終了後に`~/.codex/sessions/*.json`またはCLI出力を走査し、最新セッションIDを特定して保存しなければならない。
- **FR-003**: Claude CodeのセッションID取得は終了後に`~/.claude/projects/<encoded cwd>/sessions/*.jsonl`の最新ファイルを読み取り、メタデータのIDを保存しなければならない。
- **FR-004**: 「Continue」実行時、保存済みIDが存在すればCodexには`codex resume <id>`、Claude Codeには`claude --resume <id>`を渡し、存在しない場合は従来の`--last`/`-c`にフォールバックしなければならない。
- **FR-005**: 「Resume」実行時、gwtは保存済み履歴の一覧選択を行わず、ツールが提供するResume機能を起動しなければならない（sessionIdを自動補完しない）。ただし、Quick Start等でsessionIdが明示的に指定された場合は、そのIDをツールのResume引数として付与してもよい。
- **FR-006**: セッション終了時に「Session ID」「Resumeコマンド例」「保存先パス」をユーザーに表示し、必要ならコピーできるようにしなければならない。
- **FR-007**: セッション保存・読み出しが失敗してもワークフローをブロックしないこと。失敗時は警告を表示し、デフォルト起動に戻る。
- **FR-008**: セッションIDを提供しないツールでは、既存の保存ロジックを変更せず、Continue/ResumeでIDを要求しない。
- **FR-009**: Gemini CLIでは終了後に`~/.gemini/tmp/<project_hash>/chats/*.json`の最新ファイルからIDを抽出し、Continue/Resume時は`--resume <id>`を優先、ID不明時は`--resume`（latest）にフォールバックしなければならない。
- **FR-010**: ブランチ選択直後、同ブランチの最新履歴が存在する場合は前回の`toolId/model/sessionId`を提示するクイック選択を表示し、「前回設定で続きから」「前回設定で新規」「設定を選び直す」の3択を提供しなければならない。履歴が無い場合は従来のツール選択にフォールバックする。
- **FR-011**: Quick Startの表示内容はツール能力に応じて切り替えること。CodexのみReasoningレベルを表示し、他ツールでは非表示とする。また「Start new with previous settings」ではセッションIDを表示しない。
- **FR-012**: 同一ブランチで複数ツールを利用した場合、各ツールごとに直近設定（toolId/model/reasoningLevel/skipPermissions/sessionId）を保持し、Quick Startでツール別の「Resume with previous settings / Start new with previous settings」を提示する。履歴が無いツールは表示しない。
- **FR-013**: Web UIのブランチ詳細で保存済みセッションIDを表示し、Continue/Resume起動時に明示的なセッションIDを渡せるようにしなければならない。
- **FR-014**: Web UIのセッション起動APIは`resumeSessionId`を受け取り、Claude/Codexの起動引数に反映しなければならない。未指定時は既存のフォールバック挙動を維持する。
- **FR-015**: Web UIで起動したClaude/Codexセッションも終了時にセッションIDを検出し、`SessionData`の履歴へ保存しなければならない（検出失敗時は警告のみ）。
- **FR-016**: `ToolSessionEntry`に`toolVersion`フィールド（オプショナル）を追加し、使用したエージェントのバージョンを保存しなければならない。後方互換のため`null`/未定義も許容するが、起動時は`latest`として解釈する。
- **FR-017**: ブランチ一覧のツール表示を`ToolName@X.Y.Z | YYYY-MM-DD HH:mm`形式で表示しなければならない。バージョン情報がない場合は`ToolName@latest | YYYY-MM-DD HH:mm`形式で表示する。
- **FR-018**: 保存済み履歴に`toolVersion`が無い（null/未定義/空）場合、起動時は`latest`を選択し、次回保存時には`toolVersion`を`latest`として保存しなければならない。

### 主要エンティティ
- **SessionData**: `lastWorktreePath`, `lastBranch`, `lastUsedTool`, `mode`, `model`, 追加で `lastSessionId` を持つ。履歴`history[]`に`sessionId`/`toolId`/`branch`/`timestamp`を保持。
- **ToolSessionEntry**: `sessionId`, `toolId`, `toolLabel`, `branch`, `worktreePath`, `mode`, `model`, `timestamp`, `toolVersion`（オプショナル: 使用したエージェントのバージョン）。

## 成功基準 *(必須)*

### 測定可能な成果
- **SC-001**: Codex/Claude Codeの正常終了後、90%以上のケースで`SessionData`に`sessionId`が保存される（ローカルログで確認）。
- **SC-002**: Continue実行時に保存済みIDが存在する場合、100%のケースでCLI引数に該当IDが渡される。
- **SC-003**: セッション終了メッセージで再開コマンドが表示されることを手動確認できる（回帰テストスクリプトでstdoutを検査）。
- **SC-004**: 非対応ツール選択時に従来機能（新規起動）が阻害されないことを自動テストで確認。
- **SC-005**: 履歴があるブランチを選択した場合、クイック選択が表示され、そのうち「前回設定で続きから」選択時に同一ツール/モデル/セッションIDで起動フローへ進むことを統合テストで確認。
- **SC-006**: Web UIで起動したセッションでもセッションIDが保存され、ブランチ詳細で表示・Continue/Resumeで再開できることを手動確認できる。

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

- Web UIでのセッション一覧の詳細表示や一括コピー操作
- セッションIDのクラウド同期・共有機能
- カスタムツールのセッション管理実装
- Claude/Codex本体の挙動変更やセッション保持期間の延長

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- セッションIDは機密情報ではないが作業履歴に紐づくため、保存先はユーザーのホームディレクトリ配下に限定し、外部送信しない。
- 取得に失敗した場合でもスタックトレースを標準出力に流さず、DEBUGフラグ時のみ詳細ログを出す。

## 依存関係 *(該当する場合)*

- Codex CLIの再開コマンド（`codex resume <SESSION_ID>`）とセッションストレージ`~/.codex/sessions`。
- Claude Code CLIの再開コマンド（`claude --resume <session-id>`）とプロジェクト別ストレージ`~/.claude/projects/<encoded>/sessions/`。
- 既存のセッション保存ロジック（`src/config/index.ts`）とUIフロー（ExecutionModeSelector）。

## 参考資料 *(該当する場合)*

- [Codex CLI リファレンス: resume サブコマンド](https://developers.openai.com/codex/cli/reference/)
- [Claude Code CLI リファレンス: --resume / --continue フラグ](https://docs.claude.com/en/docs/claude-code/cli-usage)
- [Claude Code セッション保存場所例 (`~/.claude/projects/.../sessions/*.jsonl`)](https://www.reddit.com/r/ClaudeAI/comments/1pa0s0h/is_there_a_way_to_have_claude_code_search_the/)
- [Codex セッションファイルが `~/.codex/sessions` に保存される事例](https://github.com/openai/codex/issues/3817)
