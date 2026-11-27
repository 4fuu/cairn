class Cairn < Formula
  desc "Local, read-only index of coding-agent sessions"
  homepage "https://github.com/4fuu/cairn"
  version "2026.1127.0"
  license "MIT"
  depends_on arch: :arm64

  on_macos do
    url "https://github.com/4fuu/cairn/releases/download/v2026.1127.0/cairn-2026.1127.0-aarch64-apple-darwin.tar.gz"
    sha256 "6515e37788bcd11cb615d28fe324422def11771140e2192664cfb9c9e18600bc"
  end

  def install
    bin.install "cairn"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cairn --version")
  end
end
