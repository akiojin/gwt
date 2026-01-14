# Homebrew Formula for gwt

This directory contains the Homebrew formula for gwt (Git Worktree Manager).

## Usage

### From GitHub Releases (Pre-built Binary)

The formula downloads pre-built binaries from GitHub Releases.
To use it, you can create a Homebrew tap or install directly:

```bash
# Install from tap (recommended)
brew tap akiojin/gwt
brew install gwt

# Or install directly from formula
brew install --formula https://raw.githubusercontent.com/akiojin/gwt/main/homebrew/gwt.rb
```

### From Source (Build with Cargo)

You can also install by building from source:

```bash
cargo install --git https://github.com/akiojin/gwt.git gwt-cli
```

## Formula Maintenance

When a new version is released:

1. Update the `version` in `gwt.rb`
2. Download each binary and calculate SHA256:

   ```bash
   shasum -a 256 gwt-macos-aarch64
   shasum -a 256 gwt-macos-x86_64
   shasum -a 256 gwt-linux-aarch64
   shasum -a 256 gwt-linux-x86_64
   ```

3. Replace the `PLACEHOLDER_SHA256_*` values with actual hashes

## Supported Platforms

- macOS (Apple Silicon / ARM64)
- macOS (Intel / x86_64)
- Linux (ARM64)
- Linux (x86_64)
