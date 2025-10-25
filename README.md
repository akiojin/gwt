# @akiojin/claude-worktree

[日本語](README.ja.md)

Interactive Git worktree manager with AI tool selection (Claude Code / Codex CLI), graphical branch selection, and advanced workflow management.

## Overview

`@akiojin/claude-worktree` is a powerful CLI tool that revolutionizes Git worktree management through an intuitive interface. It seamlessly integrates with Claude Code / Codex CLI workflows, providing intelligent branch selection, automated worktree creation, and comprehensive project management capabilities.

## ✨ Key Features

- 🎯 **Interactive Branch Selection**: Navigate through local and remote branches with an elegant table-based interface
- 🌟 **Smart Branch Creation**: Create feature, hotfix, or release branches with guided prompts and automatic base branch selection
- 🔄 **Advanced Worktree Management**: Complete lifecycle management including creation, cleanup, and path optimization
- 🤖 **AI Tool Selection**: Choose between Claude Code / Codex CLI at launch, or use `--tool` (with `--` to pass arguments through to the tool)
- 🚀 **AI Tool Integration**: Launch the selected tool in the worktree (Claude Code includes permission handling and post-change flow)
- 📊 **GitHub PR Integration**: Automatic cleanup of merged pull request branches and worktrees
- 🛠️ **Change Management**: Built-in support for committing, stashing, or discarding changes after development sessions
- 📦 **Universal Package**: Install once, use across all your projects with consistent behavior
- 🔍 **Repository Statistics**: Real-time display of branch and worktree counts for better project overview

## Installation

### Global Installation

Install globally with bun:

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

````bash
# If installed globally
claude-worktree

# Or use bunx for one-time execution
bunx @akiojin/claude-worktree

### AI Tool Selection and Direct Launch

```bash
# Interactive selection (Claude / Codex)
claude-worktree

# Direct selection
claude-worktree --tool claude
claude-worktree --tool codex

# Pass tool-specific options (after "--")
claude-worktree --tool claude -- -r          # Resume in Claude Code
claude-worktree --tool codex -- resume --last  # Resume last Codex session
claude-worktree --tool codex -- resume <id>  # Resume specific Codex session
````

````

The tool presents an interactive interface with the following options:

1. **Select Existing Branch**: Choose from local or remote branches with worktree auto-creation
2. **Create New Branch**: Guided branch creation with type selection (feature/hotfix/release)
3. **Manage Worktrees**: View, open, or remove existing worktrees
4. **Cleanup Merged PRs**: Automatically remove branches and worktrees for merged GitHub pull requests

## Advanced Workflows

### Branch Creation Workflow

1. Select "Create new branch" from the main menu
2. Choose branch type (feature, hotfix, release)
3. Enter branch name with automatic prefix application
4. Select base branch from available options
5. Confirm worktree creation path
6. Automatic worktree setup and selected tool launch

### Worktree Management

- **Open Existing**: Launch the selected tool in existing worktrees
- **Remove Worktree**: Clean removal with optional branch deletion
- **Batch Operations**: Handle multiple worktrees efficiently

### GitHub Integration

- **Merged PR Cleanup**: Automatically detect and remove merged pull request branches
- **Authentication Check**: Verify GitHub CLI setup before operations
- **Remote Sync**: Fetch latest changes before cleanup operations

### Automated PR Merge

The repository includes an automated PR merge workflow that streamlines the development process:

- **Automatic Merge**: PRs are automatically merged when all CI checks (Test, Lint) pass and there are no conflicts
- **Merge Method**: Uses merge commit to preserve full commit history
- **Smart Skip Logic**: Automatically skips draft PRs, conflicted PRs, and failed CI runs
- **Target Branches**: Active for PRs targeting `main` and `develop` branches
- **Safety First**: Respects branch protection rules and requires successful CI completion

**How it works:**
1. PR is created targeting `main` or `develop`
2. CI workflows (Test, Lint) run automatically
3. When all CI checks pass and no conflicts exist, the PR is automatically merged
4. No manual intervention required - just create the PR and let CI handle the rest

**Disabling auto-merge:**
- Create PRs as drafts to prevent auto-merge: `gh pr create --draft`
- The auto-merge workflow respects this setting and will skip draft PRs

For technical details, see [specs/SPEC-cff08403/](specs/SPEC-cff08403/).

## System Requirements

- **Bun**: >= 1.0.0
- **Node.js** (optional): Recommended >= 18.0.0 when working with Node-based tooling
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
````

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
│   └── commands/        # Spec Kit slash commands
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

This project uses **semantic-release** for automated version management, changelog generation, and npm publishing. Releases are automatically triggered when pull requests are merged to the `main` branch.

### How Releases Work

1. **Commit with Conventional Commits**: Use standardized commit messages (`feat:`, `fix:`, `BREAKING CHANGE:`)
2. **Merge to Main**: PR merge triggers GitHub Actions
3. **Automatic Versioning**: semantic-release analyzes commits and determines version
   - `feat:` → minor version (1.0.0 → 1.1.0)
   - `fix:` → patch version (1.0.0 → 1.0.1)
   - `BREAKING CHANGE:` → major version (1.0.0 → 2.0.0)
4. **CHANGELOG Update**: Automatically generates CHANGELOG.md updates
5. **npm Publish**: Publishes to npm registry
6. **GitHub Release**: Creates release with notes

### Conventional Commits

semantic-release uses commit messages to determine release types:

| Type                                              | Description     | Version Impact        |
| ------------------------------------------------- | --------------- | --------------------- |
| `feat:`                                           | New feature     | minor (1.0.0 → 1.1.0) |
| `fix:`                                            | Bug fix         | patch (1.0.0 → 1.0.1) |
| `BREAKING CHANGE:`                                | Breaking change | major (1.0.0 → 2.0.0) |
| `chore:`, `docs:`, `style:`, `refactor:`, `test:` | No release      | -                     |

**Example commits**:

```bash
# Feature (minor release)
git commit -m "feat: add session management feature"

# Bug fix (patch release)
git commit -m "fix: resolve Docker path handling issue"

# Breaking change (major release)
git commit -m "feat!: require Bun 1.0+

BREAKING CHANGE: npx support removed, bunx required"
```

### Configuration Files

#### .releaserc.json

The semantic-release configuration file defines the release process:

```json
{
  "branches": ["main"],
  "tagFormat": "v${version}",
  "plugins": [
    "@semantic-release/commit-analyzer",
    "@semantic-release/release-notes-generator",
    "@semantic-release/changelog",
    "@semantic-release/npm",
    "@semantic-release/git",
    "@semantic-release/github"
  ]
}
```

For detailed configuration specifications, see [specs/SPEC-23bb2eed/data-model.md](./specs/SPEC-23bb2eed/data-model.md).

#### GitHub Actions Workflow

The release workflow (`.github/workflows/release.yml`) runs on every push to `main`:

1. Run tests (`bun run test`)
2. Build project (`bun run build`)
3. Execute semantic-release (version, changelog, publish)

### Manual Verification

To verify the release configuration locally:

```bash
# Dry-run to test configuration
bunx semantic-release --dry-run
```

### Resources

- [semantic-release Documentation](https://semantic-release.gitbook.io/)
- [Conventional Commits Specification](https://www.conventionalcommits.org/)
- [Release Process Guide](./specs/SPEC-23bb2eed/quickstart.md)

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
