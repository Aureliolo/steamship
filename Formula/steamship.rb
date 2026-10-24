# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.66/steamship-0.9.66-aarch64-apple-darwin.tar.gz"
      sha256 "c9d751c446a6c362c12c9186a3c74d7f51cadb3a850987cec4943e2194761de0"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.66/steamship-0.9.66-x86_64-apple-darwin.tar.gz"
      sha256 "f4a693bde963176e812e23366fa7a1ff6c57cb5a9746051525dbb1417295fdf9"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.66/steamship-0.9.66-x86_64-linux-musl.tar.gz"
      sha256 "c2729376d368579cb392e363b6180136d479d61257164044daeef0ec288a647f"
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
