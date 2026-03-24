### Core Models
- `DockerContextInfo`
  - `HasDockerCompose: bool`
  - `HasDockerfile: bool`
  - `HasDevContainer: bool`
  - `ComposePath: string`
  - `DockerfilePath: string`
  - `DevContainerPath: string`
  - `DetectedServices: List`
- `DevContainerConfig`
  - `Name: string`
  - `Service: string`
  - `DockerFile: string`
  - `WorkspaceFolder: string`
  - `RunArgs: List`
  - `ForwardPorts: List`
- `DockerLaunchRequest`
  - `WorktreePath: string`
  - `Branch: string`
  - `AgentType: string`
  - `ServiceName: string`
  - `UseDevContainer: bool`
  - `FallbackToHost: bool`
- `DockerLaunchResult`
  - `ContainerId: string`
  - `ExecCommand: string`
  - `State: string`
  - `Error: string`

### Service Boundary
- `IDockerService`
  - detect project docker context
  - list services
  - ensure container running
  - launch agent shell via `docker exec -it`
  - return host fallback recommendation on failure

### Integration Notes
- `DockerSettings` (`Gwt.Core.Models`) remains the user preference anchor
- PTY integration must still terminate in the same `IPtyService` abstraction used by host launches
