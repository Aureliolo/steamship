# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.34/steamship-0.9.34-aarch64-apple-darwin.tar.gz"
      sha256 "d530d894d0b121a06e6ea001333771496323b499a4a35da094bd651ba17e00c2"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.34/steamship-0.9.34-x86_64-apple-darwin.tar.gz"
      sha256 "5051ab46eeec37658162ec0ceafbe00f789174e5eb1077b750d9322424119732"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.34/steamship-0.9.34-x86_64-linux-musl.tar.gz"
      sha256 "59a90cce50c17318cf936c30e1556a823b084c30365179e33ed88d0d8b47a426"
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
