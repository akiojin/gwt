## Findings

- #1579 now owns the embedded skill workflow but still contains older assumptions and outdated ownership references.
- #1327 owns storage/API concerns.
- #1354 owns Issue tab detail rendering.
- #1643 owns search/discovery only.
- Migration is currently implemented only for local specs and needs to be treated as a broader redesign concern.

## References

- [The Complete Guide to Building Skills for Claude](https://resources.anthropic.com/hubfs/The-Complete-Guide-to-Building-Skill-for-Claude.pdf) — Anthropic 公式スキル設計ガイド。description フィールドの `[What] + [When] + [Capabilities]` 構造、トリガーフレーズの記載要件はこのガイドに準拠する。
