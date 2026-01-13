# 実装計画: コーディングエージェント起動の互換性整備（権限スキップ/起動ログ/終了復帰）

**仕様ID**: `SPEC-3b0ed29b` | **日付**: 2026-01-13 | **仕様書**: `specs/SPEC-3b0ed29b/spec.md`
**概要**: Codex/Claude/Gemini/OpenCodeの起動フローで、権限スキップの互換フラグ、起動ログの整備、異常終了の可視化、終了後の一覧復帰を満たす。

## 1. 前提・対象範囲

- Rust 実装の `gwt-cli` / `gwt-core` が対象。
- TTYの挙動を維持し、CLI出力は英語/ASCIIのみ。
- セッション保存やQuick Startの既存挙動を保持する。

## 2. 成功基準との対応

| 成功基準 | 計画での対応 |
| --- | --- |
| SC-003 | 起動失敗時のログ出力とユーザー向けメッセージを追加する |
| SC-008 | 異常終了を検知し、成功扱いにしない |
| SC-009 | OpenCodeのモデル選択にデフォルト/任意入力を常に提示する |

## 3. アーキテクチャ方針

1. Codexの権限スキップはバージョン判定ヘルパーで `--yolo` / `--dangerously-bypass-approvals-and-sandbox` を切替する。
2. 起動ログは整形ヘルパーで統一し、Working directory/Model/Reasoning/Mode/Skip/Args/Version/Execution method を出力する。
3. 起動終了時は exit classification (success/interrupted/failure) を行い、TUIへの復帰コンテキストに変換する。
4. OpenCodeモデルは固定リストに default/custom を保持し、空リストを許さない。

## 4. 実装ステップ (ハイレベル ToDo)

1. Codex権限スキップの互換フラグ判定とユニットテストを追加する。
2. 起動ログ整形ヘルパーを追加し、ログ出力を統一する。
3. 終了判定の分類とエラー表示/一覧復帰を実装する。
4. OpenCodeのモデル選択デフォルト/カスタム入力を明示し、ユニットテストで空リストを防止する。
5. `cargo build --release` でビルド確認する。

## 5. テスト戦略

- ユニットテスト: codex skip flag、起動ログ整形、終了分類、OpenCodeモデル選択。
- ビルド検証: `cargo build --release`。

## 6. リスクと軽減策

| リスク | 影響 | 軽減策 |
| --- | --- | --- |
| OSによる終了コード/シグナル差 | 異常終了の誤判定 | exit code + signal の併用判定にする |
| Codex CLI のバージョン取得失敗 | スキップフラグ誤選択 | 判定不能時は新フラグを優先する |

## 7. 次のステップ

1. `specs/SPEC-3b0ed29b/tasks.md` を更新
2. 必要なテストを追加・実行
