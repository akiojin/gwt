### EditMode tests
- docker context detection
  - `docker-compose.yml` detected
  - `Dockerfile` only detected
  - `.devcontainer/devcontainer.json` detected
  - no config returns empty context
- `DevContainerConfig` parsing
  - `service`, `workspaceFolder`, `forwardPorts`, `runArgs` are parsed
- launch request building
  - `docker exec -it` command contains selected service and worktree
  - host fallback flag is preserved

### Integration RED tests
- service selection UI drives selected docker service into launch request
- container boot failure returns fallback recommendation instead of hard crash
- devcontainer project launches agent inside container after config detection
