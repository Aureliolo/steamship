# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.57/steamship-0.9.57-aarch64-apple-darwin.tar.gz"
      sha256 "51ca1739a8b0a1530e7216b0451ffd4a0ee7af5e35364d6d9bd7d6ae2a641d18"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.57/steamship-0.9.57-x86_64-apple-darwin.tar.gz"
      sha256 "c2ef3af966494c105470024fec80ad013d49d1ec6d0189e4f4badf75f9f2c40c"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.57/steamship-0.9.57-x86_64-linux-musl.tar.gz"
      sha256 "3a0f7caa5fe90a591f8a750e3bd4c7c2bb19c371156a9e50ed40c77ff541baf5"
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
