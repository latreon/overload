class Overload < Formula
  desc "macOS terminal tool to find and kill resource-hungry processes fast"
  homepage "https://github.com/latreon/overload"
  url "https://github.com/latreon/overload/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "8ee1fcc743f06bbfd1e49cb40cbc58e1ebeb3a3e6f5a43e948027736d8bb24dc"
  license "MIT"
  head "https://github.com/latreon/overload.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "overload", shell_output("#{bin}/overload --help")
  end
end
