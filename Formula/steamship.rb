# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.27/steamship-0.9.27-aarch64-apple-darwin.tar.gz"
      sha256 "109545f8cb8860393f80722b30f4e1a003128fd06aea782989751c1bcdade0e2"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.27/steamship-0.9.27-x86_64-apple-darwin.tar.gz"
      sha256 "992f267dbf6accf3728afa626294fc44940858cc30ea70340317ba4be8d4ddc5"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.27/steamship-0.9.27-x86_64-linux-musl.tar.gz"
      sha256 "ff8affd5237524ac135d216566fca3694ff762c42eadf0e5ad04b6b75771dd28"
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
