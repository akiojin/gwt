# 機能仕様: エージェント状態の可視化

**仕様ID**: `SPEC-861d8cdf`
**作成日**: 2026-01-20
**更新日**: 2026-01-20
**ステータス**: ドラフト
**入力**: ユーザー説明: "tmuxモードで複数エージェントが起動している場合、バックグラウンドで実行中のエージェントペインでは、実はエージェントが作業終了していても把握できない。停止中のエージェントの場合、スピナーの色を赤にするなど改善したい。"

## 概要

tmuxモードで起動している複数のコーディングエージェントの状態（running/waiting_input/stopped）をリアルタイムで可視化する機能。Claude CodeのHook APIを利用してエージェント状態を取得し、他のエージェントはペイン出力からヒューリスティックに推測する。状態に応じてスピナーの色と形状を変更し、ステータスバーにも集計情報を表示する。

## 用語定義

- **running**: エージェントがアクティブに処理を実行中の状態
- **waiting_input**: エージェントがユーザー入力（権限確認、プロンプト入力など）を待機中の状態
- **stopped**: エージェントプロセスが終了、またはアイドル状態（60秒以上出力なし）の状態
- **Hook API**: Claude Codeが提供するイベント通知機能（UserPromptSubmit, PreToolUse, PostToolUse, Notification, Stop）

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - エージェント状態のリアルタイム表示 (優先度: P1)

開発者がtmuxモードで複数のエージェントを起動している際、各エージェントの現在の状態（running/waiting_input/stopped）がブランチリスト上で視覚的に確認でき、バックグラウンドで動作中のエージェントの状態も一目で把握できる。

**この優先度の理由**: 複数エージェントを並行して使用する開発者にとって、各エージェントの状態把握は作業効率に直結する。特にバックグラウンドエージェントが入力待ちや停止状態になっていることに気づかないと、貴重な時間を無駄にする。

**独立したテスト**: Claude Codeを起動し、各種イベント（UserPromptSubmit, Stop, Notification[permission_prompt]）を発生させ、UIの状態表示が適切に変化することを確認すれば検証できる。

**受け入れシナリオ**:

1. **前提条件** Claude Codeエージェントが起動してタスク実行中、**操作** ブランチリストを表示、**期待結果** 該当ブランチ行に緑色の星形スピナー（ACTIVE_SPINNER_FRAMES）が表示され、状態がrunningであることが分かる
2. **前提条件** Claude Codeエージェントが権限確認のプロンプトを表示中、**操作** ブランチリストを表示、**期待結果** 該当ブランチ行に黄色の点滅表示（500ms間隔）が表示され、状態がwaiting_inputであることが分かる
3. **前提条件** Claude CodeエージェントがStopイベントを発行して終了、**操作** ブランチリストを表示、**期待結果** 該当ブランチ行に赤色の停止アイコンが表示され、状態がstoppedであることが分かる
4. **前提条件** バックグラウンドのエージェントがwaiting_input状態、**操作** ブランチリストを表示、**期待結果** 該当ブランチ行にグレー輝度の黄色点滅が表示され、バックグラウンドでも注意が必要であることが分かる
5. **前提条件** エージェントが60秒以上出力なし、**操作** ブランチリストを表示、**期待結果** 該当ブランチ行の状態がstoppedに変化し、赤色表示になる

---

### ユーザーストーリー 2 - ステータスバーでの集計表示 (優先度: P1)

開発者がブランチリスト画面を見ている際、画面下部のステータスバーにエージェントの状態サマリーが表示され、waiting_inputのエージェント数など重要な情報が常に確認できる。

**この優先度の理由**: 多くのブランチがある場合、スクロールしないと見えないエージェントの状態も把握する必要がある。ステータスバーでの集計表示により、画面を見るだけで全体像を把握できる。

**独立したテスト**: 複数のエージェントを異なる状態で起動し、ステータスバーに正しいカウントが表示されることを確認すれば検証できる。

**受け入れシナリオ**:

1. **前提条件** running状態のエージェント2つ、waiting_input状態のエージェント1つが起動中、**操作** ブランチリスト画面を表示、**期待結果** ステータスバーに「2 running | 1 waiting」のような集計が表示される
2. **前提条件** waiting_input状態のエージェントが存在、**操作** ブランチリスト画面を表示、**期待結果** ステータスバーのwaiting表示部分が黄色で強調される
3. **前提条件** エージェントが1つも起動していない、**操作** ブランチリスト画面を表示、**期待結果** ステータスバーは表示されないか、「No agents」と表示される

---

### ユーザーストーリー 3 - gwt hookコマンドによる状態更新 (優先度: P1)

開発者がClaude CodeのHook設定を行った後、エージェントの各種イベント発生時に自動的にgwtのセッションファイルが更新され、状態が正確に反映される。

**この優先度の理由**: 状態検出の正確性は本機能の根幹であり、Hook APIを通じた状態更新がなければ、停止やwaiting_input状態を正確に検出できない。

**独立したテスト**: Claude CodeのHookイベントを手動でシミュレートし、.gwt-session.tomlの状態フィールドが正しく更新されることを確認すれば検証できる。

**受け入れシナリオ**:

1. **前提条件** Claude CodeがUserPromptSubmitイベントを発行、**操作** Hookがgwt hookコマンドを呼び出す、**期待結果** 該当worktreeの.gwt-session.tomlのstatusがrunningに更新される
2. **前提条件** Claude CodeがStopイベントを発行、**操作** Hookがgwt hookコマンドを呼び出す、**期待結果** 該当worktreeの.gwt-session.tomlのstatusがstoppedに更新される
3. **前提条件** Claude Codeがpermission_prompt通知を発行、**操作** Hookがgwt hookコマンドを呼び出す、**期待結果** 該当worktreeの.gwt-session.tomlのstatusがwaiting_inputに更新される
4. **前提条件** Claude CodeがPreToolUseイベントを発行、**操作** Hookがgwt hookコマンドを呼び出す、**期待結果** 該当worktreeの.gwt-session.tomlのstatusがrunningに更新される

---

### ユーザーストーリー 4 - 初回起動時のHookセットアップ提案 (優先度: P2)

開発者がgwtを初めて起動した際、Claude CodeのHook設定が行われていなければ、自動設定を提案するダイアログが表示され、ワンクリックで設定を完了できる。

**この優先度の理由**: Hook設定は本機能の前提条件だが、手動設定は煩雑であるため、自動化により導入障壁を下げる。

**独立したテスト**: ~/.claude/settings.jsonにgwt hookが登録されていない状態でgwtを起動し、設定提案ダイアログが表示されることを確認すれば検証できる。

**受け入れシナリオ**:

1. **前提条件** ~/.claude/settings.jsonが存在しない、**操作** gwtを起動、**期待結果** Hook設定の提案ダイアログが表示される
2. **前提条件** settings.jsonにgwt hookが未登録、**操作** 提案ダイアログで「設定する」を選択、**期待結果** settings.jsonにgwt hook設定が追加される
3. **前提条件** settings.jsonにgwt hookが登録済み、**操作** gwtを起動、**期待結果** 提案ダイアログは表示されない
4. **前提条件** settings.jsonにgwt hookが未登録、**操作** 提案ダイアログで「スキップ」を選択、**期待結果** ダイアログが閉じ、設定は行われない

---

### ユーザーストーリー 5 - 他エージェント（Codex, Aider等）の状態推測 (優先度: P2)

開発者がClaude Code以外のエージェント（Codex, Aider, Gemini等）を使用している場合、tmuxペインの出力パターンからヒューリスティックに状態を推測し、可能な限り正確な状態表示を行う。

**この優先度の理由**: Claude Code以外のエージェントはHook APIを持たないため、完璧な状態検出はできないが、プロセス生存確認とペイン出力解析により、ある程度の精度で状態を表示できることは有用。

**独立したテスト**: Codex CLIを起動し、プロンプト表示時とコマンド実行時でペイン出力が変化した際、状態推測が適切に変化することを確認すれば検証できる。

**受け入れシナリオ**:

1. **前提条件** Codexエージェントが起動中でコマンド実行中、**操作** ブランチリストを表示、**期待結果** プロセス生存確認により状態がrunningと表示される
2. **前提条件** Codexエージェントのプロセスが終了、**操作** ブランチリストを表示、**期待結果** プロセス終了検出により状態がstoppedと表示される
3. **前提条件** Aiderエージェントが60秒以上出力なし、**操作** ブランチリストを表示、**期待結果** アイドル検出により状態がstoppedと表示される
4. **前提条件** エージェントのペイン出力末尾にプロンプト文字列（>, →等）がある、**操作** ブランチリストを表示、**期待結果** 状態がwaiting_inputと推測される

---

## 機能要件

### FR-100: エージェント状態のデータモデル

- **FR-100a**: Session構造体に`status`フィールドを追加する。値は`running`, `waiting_input`, `stopped`の3種類とする
- **FR-100b**: Session構造体に`last_activity_at`フィールドを追加し、最後の出力/イベント時刻を記録する
- **FR-100c**: `last_activity_at`から60秒以上経過した場合、statusを`stopped`に自動更新する

### FR-101: gwt hookサブコマンド

- **FR-101a**: `gwt hook <event_name>`サブコマンドを追加する
- **FR-101b**: 標準入力からJSONペイロード（session_id, cwd, tty, event_name, notification_typeなど）を受け取る
- **FR-101c**: イベントに応じて該当worktreeの.gwt-session.tomlを更新する
- **FR-101d**: 対応イベント: UserPromptSubmit, PreToolUse, PostToolUse, Notification, Stop

### FR-102: Claude Code Hook設定

- **FR-102a**: ~/.claude/settings.jsonにgwt hook設定を追加する機能を提供する
- **FR-102b**: 初回起動時にHook未設定を検出し、設定を提案するダイアログを表示する
- **FR-102c**: Hook設定例: `{"hooks": {"UserPromptSubmit": "gwt hook UserPromptSubmit", ...}}`

### FR-103: 状態表示のUI

- **FR-103a**: running状態: 緑色の星形スピナー（ACTIVE_SPINNER_FRAMES）を表示
- **FR-103b**: waiting_input状態: 黄色のアイコンを500ms間隔で点滅表示
- **FR-103c**: stopped状態: 赤色の停止アイコン（■など）を表示
- **FR-103d**: バックグラウンドエージェントは輝度を下げた色で表示（アクティブとの区別）
- **FR-103e**: アクティブペインでもstoppedなら赤色表示（一貫性重視）

### FR-104: ステータスバー

- **FR-104a**: ブランチリスト画面の下部に1行のステータスバーを常時表示する
- **FR-104b**: ステータスバーにエージェント状態の集計を表示（例: "2 running | 1 waiting"）
- **FR-104c**: waiting_inputのカウントが1以上の場合、該当部分を黄色で強調表示する

### FR-105: 他エージェントの状態推測

- **FR-105a**: プロセス生存確認（kill -0）により、stopped/runningの基本判定を行う
- **FR-105b**: 60秒間ペイン出力がない場合、stoppedと推測する
- **FR-105c**: ペイン出力末尾にプロンプトパターン（>, →, Input:など）がある場合、waiting_inputと推測する
- **FR-105d**: 推測の精度はHook APIより低いことをドキュメントに明記する

## 非機能要件

### NFR-100: パフォーマンス

- **NFR-100a**: ペイン状態のポーリング間隔は1秒を維持する
- **NFR-100b**: スピナーアニメーションは250ms間隔を維持する
- **NFR-100c**: 点滅表示は500ms間隔とする
- **NFR-100d**: gwt hookコマンドの実行時間は100ms以内とする

### NFR-101: 互換性

- **NFR-101a**: 既存の.gwt-session.tomlとの後方互換性を保つ（statusフィールドがなくてもエラーにしない）
- **NFR-101b**: Claude Code v1.0.3以上のHook APIに対応する

## 技術設計

### データ構造

```rust
// crates/gwt-core/src/config/session.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AgentStatus {
    #[default]
    Unknown,
    Running,
    WaitingInput,
    Stopped,
}

// Session構造体に追加
pub status: AgentStatus,
pub last_activity_at: Option<DateTime<Utc>>,
```

### Hook設定例

```json
// ~/.claude/settings.json
{
  "hooks": {
    "UserPromptSubmit": "gwt hook UserPromptSubmit",
    "PreToolUse": "gwt hook PreToolUse", 
    "PostToolUse": "gwt hook PostToolUse",
    "Notification": "gwt hook Notification",
    "Stop": "gwt hook Stop"
  }
}
```

### 状態遷移ルール

| イベント | 新しいstatus |
|---------|-------------|
| UserPromptSubmit | running |
| PreToolUse | running |
| PostToolUse | running |
| Notification(permission_prompt) | waiting_input |
| Stop | stopped |
| 60秒間出力なし | stopped |

## 実装ノート

- claude-code-monitor (https://github.com/onikan27/claude-code-monitor) の実装を参考にする
- 他エージェントの状態推測は精度が低いため、「推測」であることをUI上で区別する（例: アイコンに?マークを付けるなど）ことを検討
- ステータスバーは現在のBranchListScreenには存在しないため、新規追加が必要
