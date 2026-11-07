# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.90/steamship-0.9.90-aarch64-apple-darwin.tar.gz"
      sha256 "76ea2399394d74370b9da6e2202251ba13e654e4f5ec3c60725f011333831a15"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.90/steamship-0.9.90-x86_64-apple-darwin.tar.gz"
      sha256 "3732bc58bd3ccaa9436872d0c53a977046a3198785d34ebd9440d62ce4f0c1c3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.90/steamship-0.9.90-x86_64-linux-musl.tar.gz"
      sha256 "1769cd689dc06bd744c6c16bd25c35669eaf921b76cace4e0c97ea22b22602f8"
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
