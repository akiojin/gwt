1. `plugins/gwt/skills/gwt-pr/SKILL.md` の post-merge 判定を `gwt-pr-check` と同じ deterministic ルールへ揃える。
2. `plugins/gwt/commands/gwt-pr.md` / `gwt-pr-check.md` に upstream-first fallback と `MANUAL CHECK` 条件を明記する。
3. project-local `.codex/skills/gwt-pr/SKILL.md` も同内容に同期し、現在の agent 実行文面を一致させる。
4. `crates/gwt-core/src/config/skill_registration.rs` の managed asset 展開テストで ancestor check / upstream-first fallback / manual-check 終端を固定化する。
