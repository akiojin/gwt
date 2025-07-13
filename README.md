# @akiojin/claude-worktree

[日本語](README.ja.md)

Interactive Git worktree manager for Claude Code with graphical branch selection and advanced workflow management.

## Overview

`@akiojin/claude-worktree` is a powerful CLI tool that revolutionizes Git worktree management through an intuitive interface. It seamlessly integrates with Claude Code development workflows, providing intelligent branch selection, automated worktree creation, and comprehensive project management capabilities.

## ✨ Key Features

- 🎯 **Interactive Branch Selection**: Navigate through local and remote branches with an elegant table-based interface
- 🌟 **Smart Branch Creation**: Create feature, hotfix, or release branches with guided prompts and automatic base branch selection
- 🔄 **Advanced Worktree Management**: Complete lifecycle management including creation, cleanup, and path optimization
- 🚀 **Claude Code Integration**: Seamless launch with permission configuration and post-development change handling
- 📊 **GitHub PR Integration**: Automatic cleanup of merged pull request branches and worktrees
- 🛠️ **Change Management**: Built-in support for committing, stashing, or discarding changes after development sessions
- 📦 **Universal Package**: Install once, use across all your projects with consistent behavior
- 🔍 **Repository Statistics**: Real-time display of branch and worktree counts for better project overview

## Installation

### Global Installation (Recommended)

Install globally via npm for permanent access:

```bash
npm install -g @akiojin/claude-worktree
```

### One-time Usage with npx

Run without installation using npx:

```bash
npx @akiojin/claude-worktree
```

## Quick Start

Run in any Git repository:

```bash
# If installed globally
claude-worktree

# Or use npx for one-time execution
npx @akiojin/claude-worktree
```

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
6. Automatic worktree setup and Claude Code launch

### Worktree Management

- **Open Existing**: Launch Claude Code in existing worktrees
- **Remove Worktree**: Clean removal with optional branch deletion
- **Batch Operations**: Handle multiple worktrees efficiently

### GitHub Integration

- **Merged PR Cleanup**: Automatically detect and remove merged pull request branches
- **Authentication Check**: Verify GitHub CLI setup before operations
- **Remote Sync**: Fetch latest changes before cleanup operations

## System Requirements

- **Node.js**: >= 18.0.0
- **Git**: Latest version with worktree support
- **Claude Code**: For optimal development experience
- **GitHub CLI**: Required for PR cleanup features (optional)

## Project Structure

```
@akiojin/claude-worktree/
├── src/
│   ├── index.ts          # Main application entry point
│   ├── git.ts           # Git operations and branch management
│   ├── worktree.ts      # Worktree creation and management
│   ├── claude.ts        # Claude Code integration
│   ├── github.ts        # GitHub CLI integration
│   ├── utils.ts         # Utility functions and error handling
│   └── ui/              # User interface components
│       ├── display.ts   # Console output formatting
│       ├── prompts.ts   # Interactive prompts
│       ├── table.ts     # Branch table generation
│       └── types.ts     # TypeScript type definitions
├── bin/
│   └── claude-worktree.js # Executable wrapper
└── dist/                # Compiled JavaScript output
```

## Development

### Setup

```bash
# Clone the repository
git clone https://github.com/akiojin/claude-worktree.git
cd claude-worktree

# Install dependencies
npm install

# Build the project
npm run build
```

### Available Scripts

```bash
# Development mode with auto-rebuild
npm run dev

# Production build
npm run build

# Type checking
npm run type-check

# Code linting
npm run lint

# Clean build artifacts
npm run clean

# Test the CLI locally
npm run start
```

### Development Workflow

1. **Fork and Clone**: Fork the repository and clone your fork
2. **Create Branch**: Use the tool itself to create a feature branch
3. **Development**: Make changes with TypeScript support
4. **Testing**: Test CLI functionality with `npm run start`
5. **Quality Checks**: Run `npm run type-check` and `npm run lint`
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

## Troubleshooting

### Common Issues

**Permission Errors**: Ensure Claude Code has proper directory permissions  
**Git Worktree Conflicts**: Use the cleanup feature to remove stale worktrees  
**GitHub Authentication**: Run `gh auth login` before using PR cleanup features  
**Node Version**: Verify Node.js >= 18.0.0 with `node --version`

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