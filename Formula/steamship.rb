# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.37/steamship-0.9.37-aarch64-apple-darwin.tar.gz"
      sha256 "503ace6efad49d45f34ca4d793633fc33de67eaa143726e0bd04a69877a27529"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.37/steamship-0.9.37-x86_64-apple-darwin.tar.gz"
      sha256 "6d37ecb736ac59021a4959d978c7fc83f88b9fb4e885d26b6693a01b6f49ac37"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.37/steamship-0.9.37-x86_64-linux-musl.tar.gz"
      sha256 "6920202f4d48592ef2036ead3254202a5795bcd73a70f70254852f46672f31fe"
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
