### EditMode tests
- `BuildIndexAsync()` indexes files under project root and skips ignored directories
- `Search(query)` matches file names case-insensitively
- `RefreshAsync()` rebuilds index
- `IndexedFileCount` tracks current index size
- unauthorized directory access is skipped without crashing

### Integration RED tests
- GitHub issue index build and search
- semantic search ranking once embedding backend is introduced
- background indexing progress/status reporting in HUD
