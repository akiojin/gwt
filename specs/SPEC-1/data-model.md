# Data Model: SPEC-1 - Terminal Emulation

## Primary Objects
- **Renderer surface** - `renderer.rs` maps visible vt100 cells into ratatui spans and style flags.
- **Session state** - `VtState` and transcript-backed scroll state define what the renderer can inspect.
- **URL overlay** - A URL-region map is the expected bridge between rendered coordinates and Ctrl+click handling.
- **Alt-screen fixture** - Main-screen content, alt-screen content, and cursor restore behavior form the verification boundary.
