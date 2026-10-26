# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.69/steamship-0.9.69-aarch64-apple-darwin.tar.gz"
      sha256 "064adf1cdf34a7625d291735725eba8b2bd2b2251b71a543cb01612a2da19daa"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.69/steamship-0.9.69-x86_64-apple-darwin.tar.gz"
      sha256 "8986d90da9311b3b66161fa5acf2896b19842dda86e0a514a7888692ec287161"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.69/steamship-0.9.69-x86_64-linux-musl.tar.gz"
      sha256 "2dc795d3327e56bd4b477a405aeb8e8c739093e315df3b81ca9222c1bdf28dbf"
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
