# frozen_string_literal: true

# Homebrew formula for gwt - Git Worktree Manager
class Gwt < Formula
  desc "Manage Git worktrees with AI coding agent integration"
  homepage "https://github.com/akiojin/gwt"
  version "5.0.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/akiojin/gwt/releases/download/v#{version}/gwt-macos-aarch64"
      sha256 "PLACEHOLDER_SHA256_MACOS_ARM64"
    end

    on_intel do
      url "https://github.com/akiojin/gwt/releases/download/v#{version}/gwt-macos-x86_64"
      sha256 "PLACEHOLDER_SHA256_MACOS_X64"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/akiojin/gwt/releases/download/v#{version}/gwt-linux-aarch64"
      sha256 "PLACEHOLDER_SHA256_LINUX_ARM64"
    end

    on_intel do
      url "https://github.com/akiojin/gwt/releases/download/v#{version}/gwt-linux-x86_64"
      sha256 "PLACEHOLDER_SHA256_LINUX_X64"
    end
  end

  def install
    binary_name = "gwt-#{OS.kernel_name.downcase}-#{Hardware::CPU.arch == :arm64 ? "aarch64" : "x86_64"}"
    binary_name += ".exe" if OS.windows?
    bin.install Dir["*"].find { |f| File.executable?(f) } => "gwt"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/gwt --version")
  end
end
