# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.5/steamship-0.8.5-aarch64-apple-darwin.tar.gz"
      sha256 "66450b92d628150f448a6f4b1c52595c2d6c23a6fa06b495bb92861dd422a400"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.5/steamship-0.8.5-x86_64-apple-darwin.tar.gz"
      sha256 "5d7e5552fed814a23998b8f161efc6db88aef0630b1365373f1bacd631a91627"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.5/steamship-0.8.5-x86_64-linux-musl.tar.gz"
      sha256 "31ceb7632ecd7c103c599cf6ed39756e18a36c25be4dc2afe639388ecd094255"
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
