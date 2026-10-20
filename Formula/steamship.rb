# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.40/steamship-0.9.40-aarch64-apple-darwin.tar.gz"
      sha256 "72ece02b270ab86e9358a8ab2a7018fe7bde58de6f54affce36d10c7a6bd3791"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.40/steamship-0.9.40-x86_64-apple-darwin.tar.gz"
      sha256 "0df3d66602dd6ccafdf5d925b9858a1f88b512126f83e419a4e18e10f3f0971a"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.40/steamship-0.9.40-x86_64-linux-musl.tar.gz"
      sha256 "03b7771ce9bc694feaa94e378ecc5ef5b5c4556743a8b12142b857314c8bc51a"
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
