# Research: SPEC-1 - Terminal Emulation

## Context
- The vt100 renderer already covers ANSI colors, text attributes, scrollback, and selection.
- URL handling should stay as a visible-line overlay so the core vt100 pipeline stays unchanged.
- Browser opening needs a small opener boundary that can be tested without shelling out in unit tests.
- Alt-screen confidence should come from DECSET 1049 and DECRST 1049 fixtures, not manual terminal checks.
