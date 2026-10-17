# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.21/steamship-0.9.21-aarch64-apple-darwin.tar.gz"
      sha256 "cca8e892608de51b58361d7d3618bf4fdcfee2e0dab90056a32a521c218098eb"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.21/steamship-0.9.21-x86_64-apple-darwin.tar.gz"
      sha256 "38746c4fe9b0835496b8488de86874203da6e52726ba2740fc9b4503aeead995"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.21/steamship-0.9.21-x86_64-linux-musl.tar.gz"
      sha256 "d9bc24c42867194799160cc1c04a4c5e6ceaf8e56e1ccb88345b848382bada36"
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
