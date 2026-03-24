### RED テスト方針
- `GamificationService` のユニットテストを追加し、経験値加算、複数回レベルアップ、バッジ解放、未解放バッジ判定を先に RED で固定する。
- `StudioLevel` と Agent 上限連動のテストを追加し、コミット数増加で `GetMaxAgents()` が段階的に増えることを確認する。具体的にはLv1=3, Lv5=5, Lv10=10, Lv20=無制限のテーブルを検証する。
- レベルカーブのテストを追加し、Lv2到達に10コミット程度必要なこと、上位レベルが指数的に増加することを確認する。
- 永続化テストを追加し、実績・レベル・経験値が JSON で round-trip することを確認する。
- UI 統合は別 issue へ譲り、ここではサービス層の状態遷移とデータ整合性を中心に RED を作る。

### 受け入れテストケース
- `AddExperience` で `CurrentLevel.Experience` が増加し、閾値超過時に `Level` が上がる。
- レベルアップ後は `ExperienceToNextLevel` が次レベル閾値へ更新される。
- Lv2到達に必要な経験値が10コミット相当であること。
- 初回経験値付与で初期バッジが解放され、`CheckBadge(id)` が true になる。
- `GetBadges()` は外部変更から内部状態を守るコピーを返す。
- `StudioLevel.GetMaxAgents()` がLv1=3, Lv5=5, Lv10=10, Lv20=無制限を返す。
- Lv2〜Lv4では `GetMaxAgents()` がLv1と同じ3を返す（次の閾値Lv5まで変わらない）。
