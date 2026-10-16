# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.13/steamship-0.9.13-aarch64-apple-darwin.tar.gz"
      sha256 "70436ebbebcc6d71e9a6bb5896d9057b0ac43751df4aa8681d2b25ca77478b04"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.13/steamship-0.9.13-x86_64-apple-darwin.tar.gz"
      sha256 "f35f87ace4bf1cd0232a047e5834bab399e9145428c308e1218874023bcc06c8"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.13/steamship-0.9.13-x86_64-linux-musl.tar.gz"
      sha256 "a220f177117e315ab69f634b503ab0508d90f4137bad22c75f262e9c5e4f923a"
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
