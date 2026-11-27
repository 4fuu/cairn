class Cairn < Formula
  desc "Local, read-only index of coding-agent sessions"
  homepage "https://github.com/4fuu/cairn"
  version "2026.1127.5"
  license "MIT"
  depends_on arch: :arm64

  on_macos do
    url "https://github.com/4fuu/cairn/releases/download/v2026.1127.5/cairn-2026.1127.5-aarch64-apple-darwin.tar.gz"
    sha256 "778b6563f9e3aeedaa549348c3570f6357e7ff414215a8b7848a10c7a55992ca"
  end

  def install
    bin.install "cairn"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cairn --version")
  end
end
