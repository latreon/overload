class Overload < Formula
  desc "macOS terminal tool to find and kill resource-hungry processes fast"
  homepage "https://github.com/latreon/overload"
  url "https://github.com/latreon/overload/archive/refs/tags/v0.5.0.tar.gz"
  sha256 "8ec798bcb6a62aa7fbb0c405ecffe7ce249627b5c136bd9383833e3cdf6d2852"
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
