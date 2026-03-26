### Phase 1: gpt-5.4 モデル追加（完了）
- [x] Codex モデル一覧に `gpt-5.4` が含まれる
- [x] Codex の既定モデルが version gate 付きで切り替わる
- [x] `gpt-5.4` 実効時だけ context override が付く
- [x] `gpt-5.4` 以外では context override が付かない
- [x] 既存モデル選択は維持される
- [x] GUI / core テストが GREEN

### Phase 2: Fast mode 対応（完了）
- [x] `gpt-5.4` 選択時に Fast mode チェックボックスが表示される
- [x] `gpt-5.4` 以外のモデルでは Fast mode チェックボックスが非表示
- [x] Fast mode ON で `-c service_tier=fast` が起動引数に含まれる
- [x] Fast mode OFF で `service_tier` が起動引数に含まれない
- [x] Fast mode の選択状態が Launch defaults として保存/復元される
- [x] GUI / core テストが GREEN

### Phase 3: multi-agent 親仕様化と Codex モデル一覧更新（完了）
- [x] 本 Issue が Codex 専用ではなく multi-agent のモデル管理親仕様として読める
- [x] Codex モデル一覧に `gpt-5.4-mini` が追加され、最新順序に更新される
- [x] 既存の `gpt-5.4` 固有 override / Fast mode 挙動が維持される
- [x] 対象 GUI / core テストが GREEN

### 永続管理
- [x] 本 Issue が今後の Codex / Claude Code / Gemini などのモデル更新先として利用できる（クローズしない）
