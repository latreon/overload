class Overload < Formula
  desc "macOS terminal tool to find and kill resource-hungry processes fast"
  homepage "https://github.com/latreon/overload"
  url "https://github.com/latreon/overload/archive/refs/tags/v0.4.0.tar.gz"
  sha256 "12cdc4c2a91493a3dc4ecb069d51a4a28039fbcce440d5d0d32e4c6860e3b922"
  license "MIT"
  head "https://github.com/latreon/overload.git", branch: "main"

  depends_on "rust" => :build
  depends_on :macos

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "overload", shell_output("#{bin}/overload --help")
  end
end
