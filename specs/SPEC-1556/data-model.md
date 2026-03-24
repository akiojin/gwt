### Core Models
- `MigrationState`
  - `NotNeeded`, `Available`, `InProgress`, `Completed`, `Failed`
- `MigrationPlan`
  - `ProjectRoot: string`
  - `GwtDir: string`
  - `BackupDir: string`
  - `TomlFiles: List`
  - `JsonTargets: List`
- `MigrationResult`
  - `State: MigrationState`
  - `ConvertedFiles: List`
  - `SkippedFiles: List`
  - `PreservedOriginals: List`
  - `ErrorMessage: string`
  - `BackupDir: string`
- `TomlToJsonMapping`
  - `SourcePath: string`
  - `DestinationPath: string`
  - `BackupPath: string`
  - `FileKind: string`

### Current Implementation Anchor
- `MigrationService` scans `/.gwt` for `.toml` files
- backup dir format: `backup_yyyyMMdd_HHmmss`
- converted outputs are written beside source TOML as `.json`
- **元のTOMLファイルは削除しない（保持する）**
