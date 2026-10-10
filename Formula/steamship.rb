# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.3/steamship-0.8.3-aarch64-apple-darwin.tar.gz"
      sha256 "e09906418883fdd78a32f82a98ee1cc86c5b5af95c9160ba088d80f829f0814e"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.3/steamship-0.8.3-x86_64-apple-darwin.tar.gz"
      sha256 "fb7cf15e00f3cf8242405e76463ba49a8ce74f98312aeb013dc2f0f1aa3cff53"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.3/steamship-0.8.3-x86_64-linux-musl.tar.gz"
      sha256 "e35b160289d487c64d2ff997aaed873944e0b308098387042179700c215a8b3b"
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
