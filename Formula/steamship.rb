# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.30/steamship-0.9.30-aarch64-apple-darwin.tar.gz"
      sha256 "4babdfc5fc5b555998591df8e12b9c21c0f9bedc5f5ec86c2ca12317e9a959cf"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.30/steamship-0.9.30-x86_64-apple-darwin.tar.gz"
      sha256 "2a9e48f1b6ffafb2e124bc96130805cef87cc629a9f77c40ef0855a469cb9e45"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.30/steamship-0.9.30-x86_64-linux-musl.tar.gz"
      sha256 "d3743ef5ba1a99d37dc7e8c73334555efba17b44e28e7f28ff3641bcdab0c485"
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
