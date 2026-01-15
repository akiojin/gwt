# @akiojin/gwt

[日本語](README.ja.md)

Interactive Git worktree manager with Coding Agent selection (Claude Code / Codex CLI / Gemini CLI / OpenCode), graphical branch selection, and advanced workflow management.

## Overview

`@akiojin/gwt` is a powerful CLI tool that revolutionizes Git worktree management through an intuitive interface. It seamlessly integrates with Claude Code / Codex CLI / Gemini CLI / OpenCode workflows, providing intelligent branch selection, automated worktree creation, and comprehensive project management capabilities.

## Migration Status

The Rust implementation covers the core CLI/TUI workflow and the Web UI (REST + WebSocket terminal). The migration from TypeScript/Bun to Rust is complete. Remaining work is focused on documentation polish and continuous improvements.

## Key Features

- **Modern TUI**: Built with Ratatui for a smooth, responsive terminal interface
- **Full-screen Layout**: Persistent header with repo context, boxed branch list, and always-visible footer with keyboard shortcuts
- **Smart Branch Creation**: Create feature, bugfix, hotfix, or release branches with guided prompts and automatic base branch selection
- **Advanced Worktree Management**: Complete lifecycle management including creation, cleanup of worktree-backed branches, and path optimization
- **Coding Agent Selection**: Choose between built-in agents (Claude Code / Codex CLI / Gemini CLI / OpenCode) or custom coding agents defined in `~/.gwt/tools.json`
- **Coding Agent Integration**: Launch the selected agent in the worktree (Claude Code includes permission handling and post-change flow)
- **GitHub PR Integration**: Automatic cleanup of merged pull request branches and worktrees
- **Change Management**: Built-in support for committing, stashing, or discarding changes after development sessions
- **Universal Package**: Install once, use across all your projects with consistent behavior

## Installation

GitHub Releases are the source of truth for prebuilt binaries. The npm/bunx wrapper automatically downloads the matching release asset on install.

### From GitHub Releases (Recommended)

Download pre-built binaries from the [Releases page](https://github.com/akiojin/gwt/releases):

- `gwt-linux-x86_64` - Linux x86_64
- `gwt-linux-aarch64` - Linux ARM64
- `gwt-macos-x86_64` - macOS Intel
- `gwt-macos-aarch64` - macOS Apple Silicon
- `gwt-windows-x86_64.exe` - Windows x86_64

```bash
# Example for Linux x86_64
curl -L https://github.com/akiojin/gwt/releases/latest/download/gwt-linux-x86_64 -o gwt
chmod +x gwt
sudo mv gwt /usr/local/bin/
```

### Via npm/bunx

Install globally or run without installation:

```bash
# Global install
npm install -g @akiojin/gwt
bun add -g @akiojin/gwt

# One-time execution
npx @akiojin/gwt
bunx @akiojin/gwt
```

### Via Cargo

Install the CLI with Cargo:

```bash
# From crates.io (recommended for Rust users)
cargo install gwt-cli

# With cargo-binstall (faster, downloads prebuilt binary)
cargo binstall gwt-cli

# From GitHub (latest development version)
cargo install --git https://github.com/akiojin/gwt --package gwt-cli --bin gwt --locked

# Or, from a local checkout
cargo install --path crates/gwt-cli

# Or run directly from source
cargo run -p gwt-cli
```

### Build from Source

```bash
# Clone the repository
git clone https://github.com/akiojin/gwt.git
cd gwt

# Build release binary
cargo build --release

# The binary is at target/release/gwt
./target/release/gwt
```

## Quick Start

Run in any Git repository:

```bash
# If installed globally or in PATH
gwt

# Or use bunx for one-time execution
bunx @akiojin/gwt
```

CLI options:

```bash
# Display help
gwt --help

# Check version
gwt --version

# List worktrees
gwt list

# Add worktree for existing branch
gwt add feature/my-feature

# Create new branch with worktree
gwt add -n feature/new-feature --base develop

# Remove worktree
gwt remove feature/old-feature

# Cleanup orphaned worktrees
gwt clean
```

The tool presents an interactive interface with the following options:

1. **Select Existing Branch**: Choose from local or remote branches with worktree auto-creation
2. **Create New Branch**: Guided branch creation with type selection (feature/bugfix/hotfix/release)
3. **Manage Worktrees**: View, open, or remove existing worktrees
4. **Cleanup Branches**: Remove merged PR branches or branches identical to their base directly from the CLI (branches without worktrees are excluded)

## Coding Agents

gwt detects agents available on PATH and lists them in the launcher.

Supported agents (built-in):

- Claude Code (`claude`)
- Codex CLI (`codex`)
- Gemini CLI (`gemini`)
- OpenCode (`opencode`)

### Custom coding agents

Custom agents are defined in `~/.gwt/tools.json` and will appear in the launcher.

Minimal example:

```json
{
  "version": "1.0.0",
  "customCodingAgents": [
    {
      "id": "aider",
      "displayName": "Aider",
      "type": "command",
      "command": "aider",
      "defaultArgs": ["--no-git"],
      "modeArgs": {
        "normal": [],
        "continue": ["--resume"],
        "resume": ["--resume"]
      },
      "permissionSkipArgs": ["--yes"],
      "env": {
        "OPENAI_API_KEY": "sk-..."
      }
    }
  ]
}
```

Notes:

- `type` supports `path`, `bunx`, or `command`.
- `modeArgs` defines args per execution mode (Normal/Continue/Resume).
- `env` is optional per-agent environment variables.

## Advanced Workflows

### Branch Strategy

This repository follows a structured branching strategy:

- **`main`**: Production-ready code. Protected branch for releases only.
- **`develop`**: Integration branch for features. All feature branches merge here.
- **`feature/*`**: New features and enhancements. **Must be based on and target `develop`**.
- **`hotfix/*`**: Critical production fixes. Based on and target `main`.
- **`release/*`**: Release preparation branches.

**Important**: When creating feature branches, always use `develop` as the base branch:

```bash
# Correct: Create feature branch from develop
git checkout develop
git pull origin develop
git checkout -b feature/my-feature

# Or use this tool which handles it automatically
gwt
# → Select "Create new branch" → "feature" → automatically uses develop as base
```

### Branch Creation Workflow

> **Important**: This workflow is intended for human developers. Autonomous agents must never create or delete branches unless a human gives explicit, task-specific instructions.

1. Select "Create new branch" from the main menu
2. Choose branch type (feature, bugfix, hotfix, release)
3. Enter branch name with automatic prefix application
4. Select base branch from available options (feature → develop, hotfix → main)
5. Confirm worktree creation path
6. Automatic worktree setup and selected tool launch

### Worktree Management

- **Open Existing**: Launch the selected tool in existing worktrees
- **Remove Worktree**: Clean removal with optional branch deletion
- **Batch Operations**: Handle multiple worktrees efficiently

### GitHub Integration

- **Branch Cleanup**: Automatically detect and remove merged pull request branches or branches that no longer differ from their base
- **Authentication Check**: Verify GitHub CLI setup before operations
- **Remote Sync**: Fetch latest changes before cleanup operations

## System Requirements

- **Rust**: Stable toolchain (for building from source)
- **Git**: Latest version with worktree support
- **Coding Agent**: At least one built-in agent or a custom coding agent should be available
- **GitHub CLI**: Required for PR cleanup features (optional)
- **bun/npm**: Required for bunx/npx execution method

## Project Structure

```text
@akiojin/gwt/
├── Cargo.toml           # Workspace configuration
├── crates/
│   ├── gwt-cli/         # CLI entry point and TUI (Ratatui)
│   ├── gwt-core/        # Core library (worktree management)
│   ├── gwt-web/         # Web server (future)
│   └── gwt-frontend/    # Web frontend (future)
├── package.json         # npm distribution wrapper
├── bin/gwt.js           # Binary wrapper script
├── scripts/postinstall.js  # Binary download script
├── specs/               # Feature specifications
└── docs/                # Documentation
```

## Development

### Setup

```bash
# Clone the repository
git clone https://github.com/akiojin/gwt.git
cd gwt

# Build the project
cargo build

# Run tests
cargo test

# Run with debug output
cargo run
```

### Available Commands

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run clippy lints
cargo clippy --all-targets --all-features -- -D warnings

# Format code
cargo fmt

# Run the CLI locally
cargo run
```

### Development Workflow

1. **Fork and Clone**: Fork the repository and clone your fork
2. **Create Branch**: Use the tool itself to create a feature branch
3. **Development**: Make changes with Rust
4. **Testing**: Test CLI functionality with `cargo run`
5. **Quality Checks**: Run `cargo clippy` and `cargo fmt --check`
6. **Pull Request**: Submit a PR with clear description

### Code Structure

- **Entry Point**: `crates/gwt-cli/src/main.rs` - Main application logic
- **Core Modules**: Git operations, worktree management in `gwt-core`
- **TUI Components**: Ratatui-based interface in `gwt-cli/src/tui/`
- **Type Safety**: Comprehensive Rust type definitions
- **Error Handling**: Robust error management with `thiserror`

## Release Process

We ship releases through release-please. End users can simply install the latest published package (via npm or the GitHub Releases tab) and rely on versioned artifacts. Maintainers who need the full workflow should read [docs/release-guide.md](./docs/release-guide.md) (日本語版: [docs/release-guide.ja.md](./docs/release-guide.ja.md)).

## Troubleshooting

### Common Issues

**Permission Errors**: Ensure proper directory permissions
**Git Worktree Conflicts**: Use the cleanup feature to remove stale worktrees
**GitHub Authentication**: Run `gh auth login` before using PR cleanup features
**Binary Not Found**: Ensure the gwt binary is in your PATH

### Debug Mode

For verbose output, set the environment variable:

```bash
GWT_DEBUG=1 gwt
```

## License

MIT - See LICENSE file for details

## Contributing

We welcome contributions! Please read our contributing guidelines:

1. **Issues**: Report bugs or request features via GitHub Issues
2. **Pull Requests**: Follow the development workflow above
3. **Code Style**: Maintain Rust best practices and existing patterns
4. **Documentation**: Update README and code comments for significant changes

### Contributors

- AI Novel Project Team
- Community contributors welcome

## Support

- **Documentation**: This README and inline code documentation
- **Issues**: GitHub Issues for bug reports and feature requests
- **Discussions**: GitHub Discussions for questions and community support
