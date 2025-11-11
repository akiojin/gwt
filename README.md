# @akiojin/claude-worktree

[日本語](README.ja.md)

Interactive Git worktree manager with AI tool selection (Claude Code / Codex CLI), graphical branch selection, and advanced workflow management.

## Overview

`@akiojin/claude-worktree` is a powerful CLI tool that revolutionizes Git worktree management through an intuitive interface. It seamlessly integrates with Claude Code / Codex CLI workflows, providing intelligent branch selection, automated worktree creation, and comprehensive project management capabilities.

## ✨ Key Features

- 🎯 **Modern React-based UI**: Built with Ink.js for a smooth, responsive terminal interface with real-time updates
- 🖼️ **Full-screen Layout**: Persistent header with statistics, scrollable branch list, and always-visible footer with keyboard shortcuts
- 🌟 **Smart Branch Creation**: Create feature, hotfix, or release branches with guided prompts and automatic base branch selection
- 🔄 **Advanced Worktree Management**: Complete lifecycle management including creation, cleanup, and path optimization
- 🤖 **AI Tool Selection**: Choose between Claude Code / Codex CLI through the interactive launcher
- 🚀 **AI Tool Integration**: Launch the selected tool in the worktree (Claude Code includes permission handling and post-change flow)
- 🔒 **Worktree Command Restriction**: PreToolUse hooks enforce worktree boundaries, blocking directory navigation, branch switching, and file operations outside the worktree
- 📊 **GitHub PR Integration**: Automatic cleanup of merged pull request branches and worktrees
- 🛠️ **Change Management**: Built-in support for committing, stashing, or discarding changes after development sessions
- 📦 **Universal Package**: Install once, use across all your projects with consistent behavior
- 🔍 **Real-time Statistics**: Live updates of branch and worktree counts with automatic terminal resize handling

## Installation

### Global Installation (Recommended)

Install globally with your preferred package manager:

#### bun (global install)

```bash
bun add -g @akiojin/claude-worktree
```

### One-time Usage

Run without installation using bunx:

#### bunx (bun)

```bash
bunx @akiojin/claude-worktree
```

## Quick Start

Run in any Git repository:

```bash
# If installed globally
claude-worktree

# Or use bunx for one-time execution
bunx @akiojin/claude-worktree
```

CLI options:

```bash
# Display help
claude-worktree --help

# Check version
claude-worktree --version
# or
claude-worktree -v
```

The tool presents an interactive interface with the following options:

1. **Select Existing Branch**: Choose from local or remote branches with worktree auto-creation
2. **Create New Branch**: Guided branch creation with type selection (feature/hotfix/release)
3. **Manage Worktrees**: View, open, or remove existing worktrees
4. **Cleanup Branches**: Remove merged PR branches or branches identical to their base directly from the CLI

## Web UI & Custom AI Tools

### Launching the Web UI

```bash
claude-worktree serve
# or
bunx @akiojin/claude-worktree serve
```

- Open <http://localhost:3000> to access the Worktree dashboard
- The branch list mirrors the CLI view, including search and worktree creation
- Detailed branch pages let you start AI tool sessions directly from the browser

### Managing Custom AI Tools

- Navigate to **Config** (top-right button on the dashboard or `/config`) to view and edit `~/.claude-worktree/tools.json`
- Add/edit tools with execution type (`path` / `bunx` / `command`), default arguments, mode-specific arguments, permission skip arguments, and environment variables
- Changes are written to the same `tools.json` file that the CLI uses, so both channels stay in sync
- When launching from the branch detail page you can:
  - Select any custom tool
  - Choose `normal` / `continue` / `resume` mode
  - Append extra arguments
  - Opt into the same `--dangerously-skip-permissions` flow as the CLI (with confirmation)

> Tip: use the Web UI to quickly iterate on custom tool definitions, then run them from either the CLI or browser without editing JSON manually.

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
claude-worktree
# → Select "Create new branch" → "feature" → automatically uses develop as base
```

### Branch Creation Workflow

> **Important**: This workflow is intended for human developers. Autonomous agents must never create or delete branches unless a human gives explicit, task-specific instructions.

1. Select "Create new branch" from the main menu
2. Choose branch type (feature, hotfix, release)
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

### Automated PR Merge

The repository includes an automated PR merge workflow that streamlines the development process:

- **Automatic Merge**: PRs are automatically merged when all CI checks (Test, Lint) pass and there are no conflicts
- **Merge Method**: Uses merge commit to preserve full commit history
- **Smart Skip Logic**: Automatically skips draft PRs, conflicted PRs, and failed CI runs
- **Target Branch**: Active for PRs targeting `develop` branch (feature integration)
- **Safety First**: Respects branch protection rules and requires successful CI completion

**How it works:**

1. PR is created targeting `develop`
2. CI workflows (Test, Lint) run automatically
3. When all CI checks pass and no conflicts exist, the PR is automatically merged to `develop`
4. Changes accumulate on `develop` until ready for release
5. Use `/release` command to merge `develop` to `main` and trigger semantic-release

**Disabling auto-merge:**

- Create PRs as drafts to prevent auto-merge: `gh pr create --draft`
- The auto-merge workflow respects this setting and will skip draft PRs

For technical details, see [specs/SPEC-cff08403/](specs/SPEC-cff08403/).

## System Requirements

- **Bun**: >= 1.0.0
- **Node.js** (optional): Recommended >= 18.0.0 when working with Node-based tooling
- **pnpm**: >= 8.0.0 (for CI/CD and Docker environments - uses hardlinked node_modules)
- **Git**: Latest version with worktree support
- **AI Tool**: At least one of Claude Code or Codex CLI should be available
- **GitHub CLI**: Required for PR cleanup features (optional)
- **Python**: >= 3.11 (for Spec Kit CLI)
- **uv**: Python package manager (for Spec Kit CLI)

## Spec-Driven Development with Spec Kit

This project uses **@akiojin/spec-kit**, a Japanese-localized version of GitHub's Spec Kit for spec-driven development workflows.

### Installing Spec Kit CLI

```bash
# Install globally with uv
uv tool install specify-cli --from git+https://github.com/akiojin/spec-kit.git

# Verify installation
specify --help
```

### Available Spec Kit Commands

Execute these commands in Claude Code to leverage spec-driven development:

- `/speckit.constitution` - Define project principles and guidelines
- `/speckit.specify` - Create feature specifications
- `/speckit.plan` - Create technical implementation plans
- `/speckit.tasks` - Generate actionable task lists
- `/speckit.implement` - Execute implementation

### Optional Quality Assurance Commands

- `/speckit.clarify` - Resolve ambiguities before planning
- `/speckit.analyze` - Validate consistency between spec, plan, and tasks
- `/speckit.checklist` - Verify requirement coverage and clarity

### Spec Kit Workflow

1. Start with `/speckit.constitution` to establish project foundations
2. Use `/speckit.specify` to define what you want to build
3. Run `/speckit.plan` to create technical architecture
4. Generate tasks with `/speckit.tasks`
5. Implement with `/speckit.implement`

For more details, see the [Spec Kit documentation](https://github.com/akiojin/spec-kit).

## Project Structure

```
@akiojin/claude-worktree/
├── src/
│   ├── index.ts          # Main application entry point
│   ├── git.ts           # Git operations and branch management
│   ├── worktree.ts      # Worktree creation and management
│   ├── claude.ts        # Claude Code integration
│   ├── codex.ts         # Codex CLI integration
│   ├── github.ts        # GitHub CLI integration
│   ├── utils.ts         # Utility functions and error handling
│   └── ui/              # User interface components
│       ├── display.ts   # Console output formatting
│       ├── prompts.ts   # Interactive prompts
│       ├── table.ts     # Branch table generation
│       └── types.ts     # TypeScript type definitions
├── bin/
│   └── claude-worktree.js # Executable wrapper
├── .claude/             # Claude Code configuration
│   ├── commands/        # Spec Kit slash commands
│   ├── settings.json    # Hook configuration
│   └── hooks/           # PreToolUse hooks for command restriction
│       ├── block-cd-command.sh        # Restricts cd commands to worktree
│       ├── block-git-branch-ops.sh    # Controls git branch operations
│       └── block-file-ops.sh          # Restricts file operations to worktree
├── .specify/            # Spec Kit scripts and templates
│   ├── memory/          # Project memory files
│   ├── scripts/         # Automation scripts
│   └── templates/       # Specification templates
├── specs/               # Feature specifications
└── dist/                # Compiled JavaScript output
```

## Development

### Setup

```bash
# Clone the repository
git clone https://github.com/akiojin/claude-worktree.git
cd claude-worktree

# Install dependencies (bun)
bun install

# Build the project (bun)
bun run build
```

### Available Scripts

```bash
# Development mode with auto-rebuild (bun)
bun run dev

# Production build (bun)
bun run build

# Type checking (bun)
bun run type-check

# Code linting (bun)
bun run lint

# Clean build artifacts (bun)
bun run clean

# Test the CLI locally (bun)
bun run start
```

### Development Workflow

1. **Fork and Clone**: Fork the repository and clone your fork
2. **Create Branch**: Use the tool itself to create a feature branch
3. **Development**: Make changes with TypeScript support
4. **Testing**: Test CLI functionality with `bun run start`
5. **Quality Checks**: Run `bun run type-check` and `bun run lint`
6. **Pull Request**: Submit a PR with clear description

### Code Structure

- **Entry Point**: `src/index.ts` - Main application logic
- **Core Modules**: Git operations, worktree management, Claude integration
- **UI Components**: Modular interface components in `src/ui/`
- **Type Safety**: Comprehensive TypeScript definitions
- **Error Handling**: Robust error management across all modules

## Integration Examples

### Custom Scripts

```bash
# Package.json script example
{
  "scripts": {
    "worktree": "claude-worktree"
  }
}
```

## Release Process

We ship releases through semantic-release. End users can simply install the latest published package (via npm or the GitHub Releases tab) and rely on versioned artifacts. Maintainers who need the full workflow should read [docs/release-guide.md](./docs/release-guide.md) (日本語版: [docs/release-guide.ja.md](./docs/release-guide.ja.md)) and the in-depth specs under `specs/SPEC-57fde06f/`.

## Troubleshooting

### Common Issues

**Permission Errors**: Ensure Claude Code has proper directory permissions
**Git Worktree Conflicts**: Use the cleanup feature to remove stale worktrees
**GitHub Authentication**: Run `gh auth login` before using PR cleanup features
**Bun Version**: Verify Bun >= 1.0.0 with `bun --version`

### Debug Mode

For verbose output, set the environment variable:

```bash
DEBUG=claude-worktree claude-worktree
```

## License

MIT - See LICENSE file for details

## Contributing

We welcome contributions! Please read our contributing guidelines:

1. **Issues**: Report bugs or request features via GitHub Issues
2. **Pull Requests**: Follow the development workflow above
3. **Code Style**: Maintain TypeScript best practices and existing patterns
4. **Documentation**: Update README and code comments for significant changes

### Contributors

- AI Novel Project Team
- Community contributors welcome

## Support

- **Documentation**: This README and inline code documentation
- **Issues**: GitHub Issues for bug reports and feature requests
- **Discussions**: GitHub Discussions for questions and community support
