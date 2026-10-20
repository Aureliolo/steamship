# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.43/steamship-0.9.43-aarch64-apple-darwin.tar.gz"
      sha256 "2429e8f9570423d955c4617fcbe329d7c7b8d9054c48f84e9164cabfcb0f1826"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.43/steamship-0.9.43-x86_64-apple-darwin.tar.gz"
      sha256 "3ad1ddea3e9325560b16a182d8c0c19fb401ed1461b1e020e4400344efc098a4"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.43/steamship-0.9.43-x86_64-linux-musl.tar.gz"
      sha256 "6758d0c4e7c37f3bc177089df858ba9ab6112e4f31774ec8fa673bbb17eef560"
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
