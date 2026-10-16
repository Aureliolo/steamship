# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.14/steamship-0.9.14-aarch64-apple-darwin.tar.gz"
      sha256 "2dbd3423095647876187e92b8a9dbc9af4b29d3d140b39371b9d75440107783e"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.14/steamship-0.9.14-x86_64-apple-darwin.tar.gz"
      sha256 "db2db57af7dbd82285aeeffb4e769c5910c874d1f9d966fa945efa74ef39b05a"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.14/steamship-0.9.14-x86_64-linux-musl.tar.gz"
      sha256 "1aa86ea75868dc57723face3da9a6bc4a667b3c87feef200b6890be5af0f70ce"
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
