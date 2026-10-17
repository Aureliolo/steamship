# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.22/steamship-0.9.22-aarch64-apple-darwin.tar.gz"
      sha256 "f5ed8bf21e07d4aa0793d9defbff6b428f6d02090802ba6736257e96453b0bb8"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.22/steamship-0.9.22-x86_64-apple-darwin.tar.gz"
      sha256 "417483b7699ec747edc94c472fa4251efb5c724a2b0a6574279e55595389fbff"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.22/steamship-0.9.22-x86_64-linux-musl.tar.gz"
      sha256 "976df65adc223b8001744fea0d59ac4b608ed647640fcc48f5d578a50b68e7e1"
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
