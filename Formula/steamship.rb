# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.29/steamship-0.9.29-aarch64-apple-darwin.tar.gz"
      sha256 "376c59113d41cf202e4fe367c04606f047e0e336c46b86315420d79dae232729"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.29/steamship-0.9.29-x86_64-apple-darwin.tar.gz"
      sha256 "9174c54fe5818316a0c73cfbfeba126ed90b9fb949a233e68427ea5647962355"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.29/steamship-0.9.29-x86_64-linux-musl.tar.gz"
      sha256 "b11a446f72f2ddc634a9caa020f4a2ae28ecab0ef5cd3c82b305b07208e7a084"
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
