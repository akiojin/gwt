### 背景

gwt は Launch Agent UI 上で、エージェントごとのモデル候補を明示的に管理している。現在の主な管理箇所は `gwt-gui/src/lib/components/AgentLaunchForm.svelte` であり、Codex / Claude Code / Gemini / Copilot のモデル候補がここで定義されている。

一方で Codex については、UI の候補一覧だけでなく、`crates/gwt-core/src/agent/codex.rs` に既定モデル、version gate、`gpt-5.4` 固有の context override、Fast mode などのモデル固有挙動も実装されている。そのため、対応モデルの更新が入るたびに UI と core の両方を同期して管理しないと、実際の CLI と gwt の挙動がずれる。

本 Issue は元々 Codex 新モデル対応の親仕様として運用していたが、今後は Codex 専用ではなく、gwt が内包する各エージェントのモデル管理仕様を統合的に扱う親仕様へ拡張する。以後、Codex / Claude Code / Gemini などのモデル追加・削除・既定値変更・モデル固有オプション変更は本 Issue を更新して管理する。

今回の変更では、親仕様の対象を multi-agent に拡張した上で、Codex の現行対応モデル一覧を更新する。2026-03-18 時点の Codex モデル要件は以下の順序とする。

1. `gpt-5.4`
2. `gpt-5.4-mini`
3. `gpt-5.3-codex`
4. `gpt-5.3-codex-spark`
5. `gpt-5.2-codex`
6. `gpt-5.2`
7. `gpt-5.1-codex-max`
8. `gpt-5.1-codex-mini`

> **Note:** 現行 gwt は Tauri v2 + Rust (gwt-core / gwt-tauri) + Svelte 5 (gwt-gui) スタックで構成されている。モデル候補の UI 管理は `gwt-gui/src/lib/components/AgentLaunchForm.svelte`、Codex 既定モデルロジックは `crates/gwt-core/src/agent/codex.rs`、起動引数への橋渡しは `crates/gwt-tauri/src/commands/terminal.rs` に実装されている。保存済み選択状態は `gwt-gui/src/lib/agentLaunchDefaults.ts` の `modelByAgent` に保持される。

#### Phase 1: gpt-5.4 対応（完了）

Codex の `gpt-5.4` 追加、既定モデルの version gate、1M context override は実装済みであり、本仕様の履歴として保持する。

#### Phase 2: Fast mode 対応（完了）

Codex の `gpt-5.4` 選択時限定 Fast mode（`-c service_tier=fast`）は実装済みであり、本仕様の履歴として保持する。

### ユーザーシナリオとテスト（受け入れシナリオ）

**US-1: Codex の現行対応モデルを UI から選択できる** [P0]
- 前提: Codex の Launch Agent フォームを開く
- 操作: Model を開く
- 期待: `gpt-5.4-mini` を含む現行対応モデル一覧が、定義済み順序で表示される

**US-2: Codex の既定モデルとモデル固有挙動が安全に維持される** [P0]
- 前提: Codex を model 未指定で起動する、または `gpt-5.4` を選択する
- 操作: Launch する
- 期待: `latest` と `0.111.0+` では既定モデルが `gpt-5.4` になり、`gpt-5.4` 実効時のみ context override / Fast mode が従来どおり動く

**US-3: Claude Code や Gemini など他エージェントのモデル管理も同じ親仕様で追跡できる** [P1]
- 前提: 将来、Claude Code や Gemini の対応モデルに変更が入る
- 操作: 本 Issue を更新して仕様・計画・タスクを追記する
- 期待: agent ごとのモデル変更を別 Issue に分散させず、本 Issue を継続利用できる

**US-4: 保存済みのモデル選択は後方互換を維持する** [P1]
- 前提: 既存の保存済み defaults や手動選択がある
- 操作: Launch Agent を開く／保存済み defaults を復元する
- 期待: 新しいモデル追加や親仕様の拡張後も、既存の model ID 保存形式は破壊されない

**US-5: モデル更新の変更手順が固定化される** [P2]
- 前提: 将来、新しいモデルやモデル固有オプションがリリースされる
- 操作: 本 Issue の Plan / Tasks / TDD / Research を更新して実装する
- 期待: 同一の親仕様で差分箇所と検証方法を継続管理できる

### 機能要件

| ID | 要件 |
|----|------|
| FR-001 | gwt はモデル選択を提供する組み込みエージェントの代表モデル候補を UI 上で明示的に管理しなければならない |
| FR-002 | 本 Issue は Codex 専用ではなく、Codex / Claude Code / Gemini などのモデル管理変更を継続的に扱う親 `gwt-spec` Issue として維持されなければならない |
| FR-003 | Codex のモデル候補一覧は `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex`, `gpt-5.3-codex-spark`, `gpt-5.2-codex`, `gpt-5.2`, `gpt-5.1-codex-max`, `gpt-5.1-codex-mini` の順で表示されなければならない |
| FR-004 | Codex の既定モデルはコード上で明示管理し、`latest` と support 対象の modern resolved version では最新モデル、古い resolved version では互換モデルへ切り替えられなければならない |
| FR-005 | モデル追加時、既存モデル候補および保存済み model ID は後方互換のため維持しなければならない |
| FR-006 | モデル候補更新時は関連する GUI / core テストを先に RED にしてから実装しなければならない |
| FR-007 | Codex の実効モデルが `gpt-5.4` の場合に限り、`-c model_context_window=1000000` と `-c model_auto_compact_token_limit=950000` を起動引数へ追加しなければならない |
| FR-008 | Codex の実効モデルが `gpt-5.4` でない場合、gwt は context 関連の override を追加してはならない |
| FR-009 | Launch Agent フォームで Codex かつモデル `gpt-5.4` が選択されている場合に限り、Fast mode チェックボックスを表示しなければならない |
| FR-010 | Fast mode が有効な場合、起動引数に `-c service_tier=fast` を追加しなければならない |
| FR-011 | Fast mode が無効な場合、起動引数に `service_tier` を含めてはならない |
| FR-012 | Fast mode の選択状態は Launch defaults として保存され、次回起動時に復元されなければならない |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | 今回の変更は親仕様のスコープ更新、Codex モデル一覧更新、関連テスト更新の最小範囲に限定する |
| NFR-002 | GUI と core の双方で既存回帰テストを維持する |
| NFR-003 | 将来の agent モデル追加時に差分箇所が特定しやすいよう、Issue の Plan / Tasks / TDD / Research を更新し続ける |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | Launch Agent の Codex モデル一覧テストが `gpt-5.4-mini` を含む最新順序の期待値で GREEN |
| SC-002 | Codex の既定引数テストが `latest` と `0.111.0+` は `--model=gpt-5.4`、古い resolved version は互換既定モデル前提で GREEN |
| SC-003 | 保存済み defaults の復元テストが、親仕様の multi-agent 化と Codex モデル追加後も破壊されない |
| SC-004 | `gpt-5.4` 明示選択時と `latest` 既定解決時の context override / Fast mode 関連テストが GREEN |
| SC-005 | 今後の Codex / Claude Code / Gemini などのモデル更新で、本 Issue を更新先として再利用できる |
