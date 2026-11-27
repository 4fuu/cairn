class Cairn < Formula
  desc "Local, read-only index of coding-agent sessions"
  homepage "https://github.com/4fuu/cairn"
  version "2026.1127.1"
  license "MIT"
  depends_on arch: :arm64

  on_macos do
    url "https://github.com/4fuu/cairn/releases/download/v2026.1127.1/cairn-2026.1127.1-aarch64-apple-darwin.tar.gz"
    sha256 "999552c34c66204e7c7725b899b93d343f51cce602401dbea8103c3943da9f2d"
  end

  def install
    bin.install "cairn"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cairn --version")
  end
end
