# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.32/steamship-0.9.32-aarch64-apple-darwin.tar.gz"
      sha256 "3e9a1d88bef99e565f912f4ca84167950b86366a83bda17116dacc7d00007bec"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.32/steamship-0.9.32-x86_64-apple-darwin.tar.gz"
      sha256 "c512c9c5d05bea2ea205d3961a052d1b104664bef13f67226f41f80579ab526d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.32/steamship-0.9.32-x86_64-linux-musl.tar.gz"
      sha256 "456abaa21c3bd05016df4f67cf4612e854d99059ebdad605266898df98508e64"
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
