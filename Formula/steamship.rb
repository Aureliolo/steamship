# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.4/steamship-0.8.4-aarch64-apple-darwin.tar.gz"
      sha256 "f9867a24ea7bc07205df2f50969c1e237c781644f69665867cf51eccfc73225a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.4/steamship-0.8.4-x86_64-apple-darwin.tar.gz"
      sha256 "a19896a92dc7d7028279564f82a9236beb9335f8b3fd9434e5afd79f335ce01c"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.4/steamship-0.8.4-x86_64-linux-musl.tar.gz"
      sha256 "6fbfadd5d0baf15a9593852cab9b511bf567d6f4c6184812c87b654ebd04c099"
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
