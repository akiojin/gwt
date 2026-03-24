### Implemented checks

- `cargo test -p gwt-core skill_registration`
- `cargo test -p gwt-tauri sessions`

### Test coverage focus

- project-local registration が Codex / Claude / Gemini へ必要 asset を書き出すこと
- shared `info/exclude` の gwt managed block が idempotent に再生成されること
- Claude / Codex / Gemini 向けの path rewrite が正しく入ること
- Issue-first sidebar task parsing が `SpecIssueSections.tasks` を使うこと
- local `specs/` が無くても scanner / sidebar 周辺が成立すること
