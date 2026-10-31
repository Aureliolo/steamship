# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.84/steamship-0.9.84-aarch64-apple-darwin.tar.gz"
      sha256 "d18056d25561917cb998cccbb446f28dfa00b957d2d8daec510b335966b45817"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.84/steamship-0.9.84-x86_64-apple-darwin.tar.gz"
      sha256 "14102fabe5bf352a27fe8172c96ca43d201ade3009fce89fd453ddd21395cc7a"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.84/steamship-0.9.84-x86_64-linux-musl.tar.gz"
      sha256 "6099310d3f03303983acae6438af4925039c66f4320e15546a7f30faa278f0d0"
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
