# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.71/steamship-0.9.71-aarch64-apple-darwin.tar.gz"
      sha256 "02099e8db9a1483b1b622af9230fc53221de70fd756e2eab83327f3f77c57c4f"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.71/steamship-0.9.71-x86_64-apple-darwin.tar.gz"
      sha256 "81281e159d92f7c3fd92b445a4afa23caf144f3c1eb00e4e3cd1569b5710588f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.71/steamship-0.9.71-x86_64-linux-musl.tar.gz"
      sha256 "c8f97954e8910663ab133be20bacd9ffdf619948bb2cef0de5162dc3a9c4a2a9"
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
