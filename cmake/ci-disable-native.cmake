# CI toolchain override: disable GGML_NATIVE to avoid ARM i8mm intrinsic
# compilation errors on macOS GitHub Actions runners.
#
# whisper-rs-sys passes CMAKE_* env vars to cmake, so we use
# CMAKE_TOOLCHAIN_FILE pointing here. The option() call in ggml respects
# cache variables set by toolchain files, preventing -mcpu=native.
set(GGML_NATIVE OFF CACHE BOOL "Disable native CPU optimizations for CI" FORCE)
