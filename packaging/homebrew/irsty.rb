# typed: strict
# frozen_string_literal: true

# A Homebrew formula that builds Irsty from a release's source tarball.
# Template: set url and sha256 for the release (see docs/PACKAGING.md), then
# put it in a tap, such as gawbul/homebrew-tap as Formula/irsty.rb.
class Irsty < Formula
  desc "Modern, keyboard-driven terminal IRC client"
  homepage "https://github.com/gawbul/irsty"
  url "https://github.com/gawbul/irsty/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "Apache-2.0"
  head "https://github.com/gawbul/irsty.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/irsty")
    man1.install "assets/man/irsty.1"
    bash_completion.install "assets/completions/irsty.bash" => "irsty"
    zsh_completion.install "assets/completions/_irsty"
    fish_completion.install "assets/completions/irsty.fish"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/irsty --version")
    (testpath/"config.toml").write shell_output("#{bin}/irsty --print-default-config")
    assert_match "is fine", shell_output("#{bin}/irsty --check-config --config #{testpath}/config.toml")
  end
end
