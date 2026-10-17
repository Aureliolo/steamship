# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.20/steamship-0.9.20-aarch64-apple-darwin.tar.gz"
      sha256 "842ca5a4ea76e42e1e1382a61a15550ad3ec7d34d27bfbdc4b96df31633ca2c1"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.20/steamship-0.9.20-x86_64-apple-darwin.tar.gz"
      sha256 "ee5a749c0e70e98aba0fa883a53a129ccc045404ff54b500acf2db94a0e61317"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.20/steamship-0.9.20-x86_64-linux-musl.tar.gz"
      sha256 "0734fb7ae3e88a8c5265391064562ffcb00a2bca66c8c054e6258e995278d93a"
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
