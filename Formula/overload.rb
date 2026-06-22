class Overload < Formula
  desc "macOS terminal tool to find and kill resource-hungry processes fast"
  homepage "https://github.com/latreon/overload"
  url "https://github.com/latreon/overload/archive/refs/tags/v0.3.0.tar.gz"
  sha256 "48ce0bc8fb16f5ee4e77aef30c5e92665f8fca540a196c42ecd24fec08e01b87"
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
