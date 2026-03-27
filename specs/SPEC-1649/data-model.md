- Keep `MergeUiState = merged | closed | checking | blocked | conflicting | mergeable`
- Keep `PrStatusInfo.mergeUiState` and `PrStatusLite.mergeUiState` payload fields unchanged
- Narrow semantics only:
  - `checking`: retry / unknown / pending required checks
  - `blocked`: failed required checks / changes requested / explicit blocking state

---
