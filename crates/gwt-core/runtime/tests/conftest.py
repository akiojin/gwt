"""Shared fixtures and path setup for Phase 8 runner tests."""

from __future__ import annotations

import os
import sys
from pathlib import Path

# Allow `import chroma_index_runner as runner` from sibling directory.
RUNTIME_DIR = Path(__file__).resolve().parent.parent
if str(RUNTIME_DIR) not in sys.path:
    sys.path.insert(0, str(RUNTIME_DIR))

# Use deterministic fake embeddings throughout the unit-test suite to avoid
# downloading the multilingual-e5-base model and to keep tests fast.
os.environ.setdefault("GWT_INDEX_FAKE_EMBEDDING", "1")

# Isolate the cooperative-yield coordinator lookup from the developer's real
# ~/.gwt/runtime/index-coordinator so in-process index builds never observe a
# live gwt instance's pending heavy claimants (Phase 70).
import tempfile as _tempfile  # noqa: E402

os.environ.setdefault(
    "GWT_INDEX_COORDINATOR_ROOT",
    _tempfile.mkdtemp(prefix="gwt-index-coordinator-test-"),
)
