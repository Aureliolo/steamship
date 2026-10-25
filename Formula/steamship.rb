# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.68/steamship-0.9.68-aarch64-apple-darwin.tar.gz"
      sha256 "27d16b505a2870fe154733dc9c06446670a626e7baa9141f46dd51f8b3f8be9f"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.68/steamship-0.9.68-x86_64-apple-darwin.tar.gz"
      sha256 "db4e3d2157356766de47791d2688355ceecb5475f5f02619d62d43bde393ae84"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.68/steamship-0.9.68-x86_64-linux-musl.tar.gz"
      sha256 "c07cbea5a61c1a06e477612da1cd1d003942ebf79c314f7241ad94978588d65f"
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
