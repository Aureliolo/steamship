# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.0/steamship-0.9.0-aarch64-apple-darwin.tar.gz"
      sha256 "eed2f0c18c204aaf9370e7ab957a97ef1672c2294ef98036295371c0f9ce4ae8"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.0/steamship-0.9.0-x86_64-apple-darwin.tar.gz"
      sha256 "d321bc6951c1926bfcbaddb4ccaa4e14eb1b467203f1d6240cba83bc791c1c19"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.0/steamship-0.9.0-x86_64-linux-musl.tar.gz"
      sha256 "de052b168d65882324f958e15139c38c758ac853538c8932989db3387b2486e2"
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
