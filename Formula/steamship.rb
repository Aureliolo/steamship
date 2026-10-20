# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.44/steamship-0.9.44-aarch64-apple-darwin.tar.gz"
      sha256 "2a845b2a8ebadf31fcbf49148975eba5f5b3aee555c0d0087bc90a5ce897809a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.44/steamship-0.9.44-x86_64-apple-darwin.tar.gz"
      sha256 "6ecd073b02842864f27ab2097de7d17f5c1f42bd6aefc782db73740eb6bb0532"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.44/steamship-0.9.44-x86_64-linux-musl.tar.gz"
      sha256 "921334f18153f35318912afa594ca84f7cf20edb31d08561d084d315617b08e6"
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
