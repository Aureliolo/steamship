# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.3/steamship-0.9.3-aarch64-apple-darwin.tar.gz"
      sha256 "d75c95c333cd74dcdf356fcc0ac9b0a9280a1b1deb424953fb68064457b67b66"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.3/steamship-0.9.3-x86_64-apple-darwin.tar.gz"
      sha256 "dc1c05b496bf5ed60e134792aff6b132f11317943b5fa66ff2d74f71d513c0a6"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.3/steamship-0.9.3-x86_64-linux-musl.tar.gz"
      sha256 "c5157ae0f489cb8cb5bf95fa4d1c26007a0b6aa677c6ab27b21260fc89b79b30"
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
