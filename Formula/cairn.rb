class Cairn < Formula
  desc "Local, read-only index of coding-agent sessions"
  homepage "https://github.com/4fuu/cairn"
  version "2026.1127.4"
  license "MIT"
  depends_on arch: :arm64

  on_macos do
    url "https://github.com/4fuu/cairn/releases/download/v2026.1127.4/cairn-2026.1127.4-aarch64-apple-darwin.tar.gz"
    sha256 "6b61ead387b3d2a5872516b11c5b73c0207c87863c37cbac005e5d2966c6b449"
  end

  def install
    bin.install "cairn"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cairn --version")
  end
end
