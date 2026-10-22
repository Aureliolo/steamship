# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.51/steamship-0.9.51-aarch64-apple-darwin.tar.gz"
      sha256 "6a7530e33517ad6b02ae823f7c057b3a6dd37cf1734d49f3dd1dbb137b45e3dd"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.51/steamship-0.9.51-x86_64-apple-darwin.tar.gz"
      sha256 "71efa9fb8c69840ff636b93e241667eb97d371f6d92b28825f035ba3c9518fa0"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.51/steamship-0.9.51-x86_64-linux-musl.tar.gz"
      sha256 "8238ba42e0a08f2e0fa42a3b86b229346066ec02864b9d7a0828923385dcd0d6"
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
