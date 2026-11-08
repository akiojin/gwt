"""Hermetic Git environments for temporary repository fixtures."""

from __future__ import annotations

import os


def git_fixture_env() -> dict[str, str]:
    env = os.environ.copy()
    for key in (
        "GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_PREFIX", "GIT_NAMESPACE",
        "GIT_COMMON_DIR", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_SYSTEM",
    ):
        env.pop(key, None)
    for key in list(env):
        if (
            key == "GIT_CONFIG_COUNT"
            or key.startswith("GIT_CONFIG_KEY_")
            or key.startswith("GIT_CONFIG_VALUE_")
        ):
            env.pop(key)
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    env["GIT_CONFIG_GLOBAL"] = os.devnull
    env["GIT_ATTR_NOSYSTEM"] = "1"
    return env
