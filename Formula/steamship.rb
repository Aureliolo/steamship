# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.87/steamship-0.9.87-aarch64-apple-darwin.tar.gz"
      sha256 "82f7d94a518cd6ba0d80fa1dfbf5f81719c0734a82734a4070961d1ad022aba9"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.87/steamship-0.9.87-x86_64-apple-darwin.tar.gz"
      sha256 "9f61c971f209c72bd8b15d199084075b8230e0d76167c3a77469ccf289e4f222"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.87/steamship-0.9.87-x86_64-linux-musl.tar.gz"
      sha256 "a2de44cbb7f595e5083b621c2b5d80701db34794bbf1e1f6176032d43a29d434"
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
