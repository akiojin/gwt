---
description: "Dependabot PR の向き先を develop に固定するためのタスク"
---

# タスク: Dependabot PR の向き先を develop に固定

**入力**: `/specs/SPEC-a0d7334d/`
**前提条件**: spec.md, plan.md
**テスト**: 設定確認のみ（自動テストなし）

## タスク

- [ ] **T001** `.github/dependabot.yml` に target-branch: "develop" を追加する
- [ ] **T002** 変更後の設定内容を確認し、受け入れ条件を満たすことを確認する
