# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.79/steamship-0.9.79-aarch64-apple-darwin.tar.gz"
      sha256 "5bb93c04dda275f048393e9cffa4f40fafba2a72d896acef02f0a7a282122c7e"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.79/steamship-0.9.79-x86_64-apple-darwin.tar.gz"
      sha256 "b18ffd57c65927d794ff194469aee3a04970ebc40d20eb5ca673a25382cf0b36"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.79/steamship-0.9.79-x86_64-linux-musl.tar.gz"
      sha256 "6e5949655e953d5ea20054358e6adcf0c92ac1af88651f9052e1f5354bece78b"
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
