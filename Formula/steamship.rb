# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.55/steamship-0.9.55-aarch64-apple-darwin.tar.gz"
      sha256 "51f3e1a3754887267f68a33ec13e96f81fbc4ec6c4f073cf1f3d30c25dc14e82"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.55/steamship-0.9.55-x86_64-apple-darwin.tar.gz"
      sha256 "cd2c6d343ecec1205dedd0f69acee09a0834a5948fe3735d7912ae54d628a8c6"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.55/steamship-0.9.55-x86_64-linux-musl.tar.gz"
      sha256 "91681f8f1eb1a34cb34257360f0a46efe38d2d6e58fd711e23c36051e6fe18d5"
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
