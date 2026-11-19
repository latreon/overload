class Overload < Formula
  desc "macOS terminal tool to find and kill resource-hungry processes fast"
  homepage "https://github.com/latreon/overload"
  url "https://github.com/latreon/overload/archive/refs/tags/v0.4.0.tar.gz"
  sha256 "0019dfc4b32d63c1392aa264aed2253c1e0c2fb09216f8e2cc269bbfb8bb49b5"
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
