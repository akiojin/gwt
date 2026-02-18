# データモデル: エージェントモード（GUI版）

## AgentModeState

- `messages`: チャット履歴
- `ai_ready`: AI設定の有効状態
- `ai_error`: AI設定エラー
- `last_error`: 直近エラー
- `is_waiting`: 実行中フラグ
- `session_name`: セッション名
- `llm_call_count`: LLM呼び出し回数
- `estimated_tokens`: 推定トークン数

## SubAgent

- `pane_id`: GUI内蔵ターミナルペインID
- `agent_type`: Claude/Codex/Gemini 等
- `status`: Starting/Running/Completed/Failed

## Tool Calls

- `send_keys_to_pane(pane_id, text)`
- `send_keys_broadcast(text)`
- `capture_scrollback_tail(pane_id, max_bytes?)`
