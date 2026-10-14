# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.5/steamship-0.9.5-aarch64-apple-darwin.tar.gz"
      sha256 "39122482e7e2560496e8d34a70e9717df1cc03261d48a8af775a235f810d4b3d"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.5/steamship-0.9.5-x86_64-apple-darwin.tar.gz"
      sha256 "36e00c9502cf7a67acbb422573a9b4ce5802bbb1d556f33c83a28c6170c68046"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.5/steamship-0.9.5-x86_64-linux-musl.tar.gz"
      sha256 "f5aed9dcbc503fcc5361f8a2d6dd7398152ebedb2f26eb4d09e8faf8d3ccce64"
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
