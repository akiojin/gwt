# 実装計画: Codex CLI gpt-5.1 デフォルト更新

**仕様ID**: `SPEC-5e0c1c49` | **日付**: 2025-11-14 | **仕様書**: [spec.md](./spec.md)
**概要**: Codex CLI 起動時に `gpt-5.1-codex` モデルを常に指定し、既定フラグとの順序・互換性を維持したまま TDD で更新する。

## 1. 前提・対象範囲

- 対象は `src/codex.ts` と `tests/unit/codex.test.ts` に限定する。
- Bun + `execa` ベースのプロセス実行ロジック、端末ハンドリング（`createChildStdio`）は既存のまま利用する。
- README / README.ja.md / src / tests から `gpt-5-codex` が消えていることを `rg "gpt-5-codex" src tests README.md README.ja.md` で確認する（spec ディレクトリは除外）。

## 2. 成功基準との対応

| 成功基準 | 計画での対応 |
| --- | --- |
| SC-001 | ユニットテストを RED→GREEN で更新し、`DEFAULT_CODEX_ARGS` の期待値を `gpt-5.1-codex` にする |
| SC-002 | `launchCodexCLI` のログに依存せずとも test double で `--model` フラグを確認できる構造を維持 |
| SC-003 | `rg "gpt-5-codex" src tests README.md README.ja.md` を実行してヒットが無いことを確かめる |

## 3. アーキテクチャ方針

1. `DEFAULT_CODEX_ARGS` は単一配列定数で集中管理しているため、この配列を更新するだけで全てのモードに伝播する。
2. テストは `@openai/codex@latest` 呼び出し時の `args` 配列を丸ごと比較しているので、配列の更新でリグレッションが検知可能。
3. CLI オプション順序は変えず、新モデルは既存位置（`--enable`/`--sandbox` の間）をそのまま利用する。

## 4. 実装ステップ (ハイレベル ToDo)

1. **テスト更新 (RED)**
   - `tests/unit/codex.test.ts` の `DEFAULT_CODEX_ARGS` 定数および関連期待値を `--model=gpt-5.1-codex` へ置換し、テストを実行して失敗を確認。
2. **実装更新 (GREEN)**
   - `src/codex.ts` の `DEFAULT_CODEX_ARGS` を同様に更新してテストを成功させる。
3. **バリデーション**
   - `bun test tests/unit/codex.test.ts` を実行。
   - `rg "gpt-5-codex" src tests README.md README.ja.md` で旧文字列が残っていないことを確認。

## 5. テスト戦略

- 単体テスト: `tests/unit/codex.test.ts` で `launchCodexCLI` の引数配列を検証。
- 静的検証: `rg` による文字列サーチで旧モデル名の残存を検出。

## 6. リスクと軽減策

| リスク | 影響 | 軽減策 |
| --- | --- | --- |
| Codex CLI が `gpt-5.1-codex` をまだ受け付けない環境 | CLI 起動失敗 | 必要なら `extraArgs` で `--model` を上書きできるので回避可能。リリースノートを参照し、CLI 依存バージョンを `@openai/codex@latest` のまま維持 |
| テストと実装の配列が乖離 | リグレッション未検出 | `DEFAULT_CODEX_ARGS` を単一箇所に定義し、テスト側も同じ配列を定義して比較する現行スタイルを継続 |

## 7. オープン事項

- 2025-11-14 時点では追加のサンドボックス構成更新は不要と判断。Codex 側要求が変わったら別仕様で扱う。
