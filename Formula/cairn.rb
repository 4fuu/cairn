class Cairn < Formula
  desc "Local, read-only index of coding-agent sessions"
  homepage "https://github.com/4fuu/cairn"
  version "2026.1127.2"
  license "MIT"
  depends_on arch: :arm64

  on_macos do
    url "https://github.com/4fuu/cairn/releases/download/v2026.1127.2/cairn-2026.1127.2-aarch64-apple-darwin.tar.gz"
    sha256 "02040bffad960d1daff488092f138b8468d4c6343a79c96a8d7d05fe4195298d"
  end

  def install
    bin.install "cairn"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cairn --version")
  end
end
