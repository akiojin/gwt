# 実装計画: Codex CLI gpt-5.2-codex 対応

**仕様ID**: `SPEC-96e694b4` | **日付**: 2025-12-18 | **仕様書**: [spec.md](./spec.md)
**概要**: Codex のモデル一覧を4種類に限定し、デフォルトモデルを gpt-5.2-codex に更新して Extra high 推論レベルを選択可能にする。

## 1. 前提・対象範囲

- 対象は Codex のモデル選択とデフォルト引数に限定する。
- 既存の起動フロー、フラグ順序、セッション処理は変更しない。
- 変更対象ファイルは `crates/gwt-cli/src/tui/screens/wizard.rs`、`crates/gwt-core/src/agent/codex.rs`、`crates/gwt-cli/src/main.rs` と関連テスト。

## 2. 成功基準との対応

| 成功基準 | 計画での対応 |
| --- | --- |
| SC-001 | モデル一覧テストを更新し、gpt-5.2-codex と Extra high を検証する |
| SC-002 | デフォルト引数のテスト更新で gpt-5.2-codex を確認する |
| SC-003 | UI モデル選択テストで4件限定と並び順を検証する |

## 3. アーキテクチャ方針

1. Codex のデフォルトモデルは単一の定数で管理し、CLI 起動と Web UI のデフォルト引数に反映する。
2. モデル選択肢は一覧配列の順序で UI 表示を決めるため、4種類に限定し最新モデルを先頭寄りに配置する。
3. 推論レベルの既定値はモデルごとに明示し、Extra high を利用可能にしつつ過度なデフォルト化を避ける。

## 4. 実装ステップ (ハイレベル ToDo)

1. **テスト更新 (RED)**
   - `crates/gwt-cli/src/tui/screens/wizard.rs` の Codex モデル一覧テストを gpt-5.2-codex と 4件限定に合わせて更新する。
   - `crates/gwt-core/src/agent/codex.rs` と `crates/gwt-cli/src/main.rs` のデフォルトモデル期待値を gpt-5.2-codex に更新する。
2. **実装更新 (GREEN)**
   - `crates/gwt-cli/src/tui/screens/wizard.rs` の Codex モデル一覧に gpt-5.2-codex を追加し、4件の並び順を調整する。
   - `crates/gwt-core/src/agent/codex.rs` のデフォルトモデルを gpt-5.2-codex に更新する。
3. **バリデーション**
   - 関連ユニットテストを実行し、失敗がないことを確認する。

## 5. テスト戦略

- ユニットテスト: `crates/gwt-cli/src/tui/screens/wizard.rs`、`crates/gwt-core/src/agent/codex.rs`、`crates/gwt-cli/src/main.rs`。

## 6. リスクと軽減策

| リスク | 影響 | 軽減策 |
| --- | --- | --- |
| Codex CLI が gpt-5.2-codex を受け付けない環境が残る | 起動失敗 | 既存のエラーメッセージは変更せず、ユーザーがモデルを上書きできる状態を維持する |
| モデル一覧の順序変更で UI テストが崩れる | テスト失敗 | 先にテストを更新し、順序が固定であることを確認する |
| テストより先に実装を変更してしまう可能性 | TDDサイクルが崩れ、テストがない状態でコードが変更される | 実装ステップ1（テスト更新）の完了を確認してから、ステップ2（実装更新）に進む |

## 7. オープン事項

- gpt-5.2-codex の既定推論レベルは高い推論を維持する方針で更新する。
