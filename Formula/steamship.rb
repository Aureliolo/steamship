# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.8/steamship-0.9.8-aarch64-apple-darwin.tar.gz"
      sha256 "2fcc61504a3b80308bd5e577e6d85f381a9a6bd8479d2370f498a565bbaa4429"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.8/steamship-0.9.8-x86_64-apple-darwin.tar.gz"
      sha256 "684ecd8fa8b603e0d42225daebb46602e06364aa70cf1b8b6c4d59b56e7890aa"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.8/steamship-0.9.8-x86_64-linux-musl.tar.gz"
      sha256 "f0e449136de32d0d7193c826777e185a9344967c77713888c86349408d941a1a"
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
