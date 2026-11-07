# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.89/steamship-0.9.89-aarch64-apple-darwin.tar.gz"
      sha256 "6c2b95215d9032d48b09e54c610c6fe5f30aba42a745e65b429c4f11c9e5da6e"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.89/steamship-0.9.89-x86_64-apple-darwin.tar.gz"
      sha256 "e6c2ca1a0ef851f4c9f2c23bd4acef4b66b9dcac2f379fc7aece04a4b6a03b47"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.89/steamship-0.9.89-x86_64-linux-musl.tar.gz"
      sha256 "2147b512a69e3333a1125bb60275d25cdae1d350cfb7c66f19149d08622769a5"
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
