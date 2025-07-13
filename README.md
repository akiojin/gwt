# claude-worktree

Interactive Git worktree manager for Claude Code with graphical branch selection.

## Overview

`claude-worktree` is an interactive CLI tool that enhances Git worktree management with a user-friendly interface. It provides cursor-based selection for branches, automatic worktree creation, and seamless integration with Claude Code development workflows.

## Features

- 🎯 **Interactive Branch Selection**: Navigate through local and remote branches with cursor-based selection
- 🌟 **Smart Branch Creation**: Create new feature, hotfix, or release branches with guided prompts
- 🔄 **Automatic Worktree Management**: Handles worktree creation, cleanup, and path management
- 🚀 **Claude Code Integration**: Optimized for Claude Code development workflows
- 📦 **Universal NPM Package**: Install once, use across all your projects

## Installation

```bash
npm install -g claude-worktree
```

## Usage

```bash
claude-worktree
```

The tool will present an interactive interface where you can:

1. Select an existing local or remote branch
2. Create a new branch (feature/hotfix/release)
3. Automatically create and switch to the corresponding worktree
4. Begin development with Claude Code

## Requirements

- Node.js >= 18.0.0
- Git with worktree support
- Claude Code (optional, but recommended)

## Development

```bash
# Clone the repository
git clone https://github.com/akiojin/claude-worktree.git
cd claude-worktree

# Install dependencies
npm install

# Build the project
npm run build

# Run in development mode
npm run dev

# Type checking
npm run type-check

# Linting
npm run lint
```

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.