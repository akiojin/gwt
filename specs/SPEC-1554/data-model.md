### Index Models
- `FileIndexEntry`
  - `RelativePath: string`
  - `FileName: string`
  - `SizeBytes: long`
  - `LastModified: string`
  - `Extension: string`
- `IssueIndexEntry`
  - `Number: int`
  - `Title: string`
  - `Body: string`
  - `Labels: List`
  - `UpdatedAt: string`
- `IndexStatus`
  - `IndexedFileCount: int`
  - `PendingFiles: int`
  - `LastIndexedAt: string`
  - `IsRunning: bool`
- `SearchResultGroup`
  - `Files: List`
  - `Issues: List`

### Service Boundary
- `IProjectIndexService`
  - `BuildIndexAsync(projectRoot)`
  - `Search(query)`
  - `RefreshAsync(projectRoot)`
  - `IndexedFileCount`
- future semantic search extension
  - embedding generation
  - issue index build
  - semantic ranking

### Index Rules
- skip dirs: `.git`, `node_modules`, `.gwt`, `Library`, `Temp`, `obj`, `bin`
- relative path is stored from project root
