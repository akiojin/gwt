# Quickstart: TUI Theme System (SPEC-11)

## Minimum validation flow

### 1. Build check

```bash
cargo build -p gwt-tui
```

### 2. Test suite

```bash
cargo test -p gwt-core -p gwt-tui
```

### 3. Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### 4. Inline reference elimination check (SC-001)

```bash
grep -rn 'Color::Yellow\|Color::Cyan\|Color::Green\|Color::Red\|Color::Gray\|Color::DarkGray' \
  crates/gwt-tui/src/screens/ crates/gwt-tui/src/widgets/
# Expected: zero matches
```

### 5. Theme module existence check (SC-004)

```bash
grep 'pub mod theme' crates/gwt-tui/src/lib.rs
# Expected: pub mod theme;
```

### 6. Visual smoke test

```bash
cargo run -p gwt-tui
```

Verify:

- Pane borders use rounded corners (`╭╮╰╯`)
- Focused pane has thick/double border
- Branch icons show `◆`/`◇` instead of `●`/`○`
- Session tabs show `›`/`◈` instead of `▶`/`⭐`
