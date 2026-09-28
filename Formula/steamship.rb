# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.1/steamship-0.6.1-aarch64-apple-darwin.tar.gz"
      sha256 "bc8905e27cbbab62ee58d155d4c02317db67db8e6ae15cb886a6b4da57ca4bc5"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.1/steamship-0.6.1-x86_64-apple-darwin.tar.gz"
      sha256 "22a83e24d62c1460e25e81f39a5b3646619981a1597cea80f04ff21f1cbf005e"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.1/steamship-0.6.1-x86_64-linux-musl.tar.gz"
      sha256 "6c7d431ad1a14baaa5b9cd606b1c2bb6614ab3191d74f2399d04223207650020"
    end
  end

  def install
    bin.install "steamship"
    man1.install Dir["man/*.1"]
    bash_completion.install "completions/steamship.bash" => "steamship"
    zsh_completion.install "completions/_steamship"
    fish_completion.install "completions/steamship.fish"
  end

  test do
    assert_equal "steamship #{version}", shell_output("#{bin}/steamship --version").strip
  end
end
