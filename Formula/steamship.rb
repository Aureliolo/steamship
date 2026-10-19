# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.35/steamship-0.9.35-aarch64-apple-darwin.tar.gz"
      sha256 "39dea51ac8a5b59945e2f0941f687509a99717f1f4cbb32afe8f0b0f2307a2d8"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.35/steamship-0.9.35-x86_64-apple-darwin.tar.gz"
      sha256 "286397156c2ea392655cf80b2c4ae771a6b21e9c341de418958e41cc63502eeb"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.35/steamship-0.9.35-x86_64-linux-musl.tar.gz"
      sha256 "11e0b193184afa7cbae9859bed086df65c354f7e105ad7c5def89bf90a006a72"
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
