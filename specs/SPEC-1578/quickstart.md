1. Rust 実装 `crates/gwt-core/src/config/skill_registration.rs` を読んでロジックを把握する
2. `Gwt.Infra` アセンブリに `SkillRegistration/` ディレクトリを作成
3. `ISkillRegistrationService` インターフェースを定義
4. `EnsureProjectLocalExcludeRules` を最初に実装（コア機能）
5. ユニットテストで冪等性・エラーハンドリングを確認
6. VContainer に DI 登録、プロジェクトオープンイベントでトリガー

---
