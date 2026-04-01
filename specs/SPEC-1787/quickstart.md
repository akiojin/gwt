# Quickstart: SPEC-1787

## Minimum Validation Flow

### 1. Bare Removal Verification

```bash
# After Phase 1 implementation:
cargo build -p gwt-core -p gwt-tui
cargo test -p gwt-core -p gwt-tui
cargo clippy --all-targets --all-features -- -D warnings

# Verify no Bare references remain:
grep -r "Bare" crates/ --include="*.rs" | grep -v "test" | grep -v "comment"
```

### 2. Initialization Flow Verification

```bash
# Create empty test directory:
mkdir /tmp/gwt-test-init && cd /tmp/gwt-test-init

# Launch gwt-tui — should show fullscreen init screen:
cargo run -p gwt-tui

# Enter a repo URL → clone → SPECs tab should appear
# Press Esc → TUI should exit
```

### 3. Pre-commit Hook Verification

```bash
# After clone, verify hook exists:
cat .git/hooks/pre-commit

# On develop, try to commit:
git checkout develop
touch test-file && git add test-file
git commit -m "test"  # Should be BLOCKED

# On feature branch, commit should work:
git checkout -b feature/test-hook
git commit -m "test"  # Should SUCCEED
```

### 4. SPEC Launch Verification

```bash
# Launch gwt-tui in a repo with specs/:
cargo run -p gwt-tui

# Navigate to SPECs tab
# Select a SPEC → press Launch Agent key
# Verify wizard opens with feature/feature-{N} pre-filled
```

### 5. SPEC Drafting Verification

```bash
# Navigate to SPECs tab (empty or populated)
# Press 'n' for New SPEC
# Verify agent launches on develop branch
# Verify agent has SPEC drafting skill context
```
