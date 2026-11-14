# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.3/steamship-1.0.3-aarch64-apple-darwin.tar.gz"
      sha256 "666f77bdcbc762f3a9979b9be2fb70961b49e260e4bfcee166bcae1fa9a47361"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.3/steamship-1.0.3-x86_64-apple-darwin.tar.gz"
      sha256 "27d6c71442f8f0b69dd8669abce8f70042286f7f616df3f73f551af251e04345"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.3/steamship-1.0.3-x86_64-linux-musl.tar.gz"
      sha256 "3ab3a5bc82c5226b8f86aa4f19d127031fa542d319ad367540288c20476a7f3a"
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
