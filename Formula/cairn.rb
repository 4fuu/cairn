class Cairn < Formula
  desc "Local, read-only index of coding-agent sessions"
  homepage "https://github.com/4fuu/cairn"
  version "2026.1127.3"
  license "MIT"
  depends_on arch: :arm64

  on_macos do
    url "https://github.com/4fuu/cairn/releases/download/v2026.1127.3/cairn-2026.1127.3-aarch64-apple-darwin.tar.gz"
    sha256 "130ddc4818cda4bba7c385eb4c294c9c4d6bed304fc6923517d2178a34fe199e"
  end

  def install
    bin.install "cairn"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cairn --version")
  end
end
