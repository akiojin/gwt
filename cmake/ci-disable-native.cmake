# CI toolchain override for macOS GitHub Actions runners.
#
# whisper-rs-sys passes CMAKE_* env vars to cmake, so we use
# CMAKE_TOOLCHAIN_FILE pointing here. Toolchain-file cache variables
# are applied before project() and take precedence over defaults.
#
# 1. Disable GGML_NATIVE to avoid ARM i8mm intrinsic errors
#    (Xcode 16.4 Apple Clang + -mcpu=native).
set(GGML_NATIVE OFF CACHE BOOL "Disable native CPU optimizations for CI" FORCE)
#
# 2. Set deployment target to macOS 11.0+.
#    ggml uses std::filesystem (requires 10.15+); ARM Macs need 11.0+.
#    cmake-rs (via cc crate) may inject -mmacosx-version-min=10.13 into
#    CMAKE_C_FLAGS, but cmake appends its own flag AFTER those, and
#    clang uses the last -mmacosx-version-min it sees.
set(CMAKE_OSX_DEPLOYMENT_TARGET "11.0" CACHE STRING "macOS 11.0+ for ARM and std::filesystem" FORCE)
