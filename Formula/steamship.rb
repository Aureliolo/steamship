# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.0/steamship-0.8.0-aarch64-apple-darwin.tar.gz"
      sha256 "9da08e26c742368c05fa71d0d862e0ce2f9931ab1bb010583ec8fce823b8fb8c"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.0/steamship-0.8.0-x86_64-apple-darwin.tar.gz"
      sha256 "15b62d3c422d1ce92cc14f4388d2c2bd3cb8358365fb0b09873f5c59a66224aa"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.0/steamship-0.8.0-x86_64-linux-musl.tar.gz"
      sha256 "6a6704b2fcfdc5139d0e32f856a6a54e838423690df22a83b5f5da5d3f8554eb"
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
