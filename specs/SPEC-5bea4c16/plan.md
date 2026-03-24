1. 認証判定ロジックを共通化し、`env + active profile.ai.api_key` の OR 判定へ拡張する
2. Launch の env 構築に `ai.api_key -> OPENAI_API_KEY` フォールバック注入を追加（既存キーは上書きしない）
3. TDD で Rust テストを先に追加（RED確認）
4. GUI E2E に Profiles API キー保存シナリオを追加
5. 実装後にテストを GREEN 化し、Issue と PR を更新する
