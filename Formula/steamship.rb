# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.83/steamship-0.9.83-aarch64-apple-darwin.tar.gz"
      sha256 "731c168b6f9ccb71053fa1c551cb9279ea4fb50f7c87f19ee0ea0b5787392a98"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.83/steamship-0.9.83-x86_64-apple-darwin.tar.gz"
      sha256 "cabe43d651c1ed3fda416a60801cd1f478aa1895bf5c27e27573b5c0ed7ba9fb"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.83/steamship-0.9.83-x86_64-linux-musl.tar.gz"
      sha256 "94fff43165bbff2e33b040397263b40073ef1dfe72bebcf11ae48eb31d0c2d1d"
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
