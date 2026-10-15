# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.10/steamship-0.9.10-aarch64-apple-darwin.tar.gz"
      sha256 "5d09c1486833cdd39d8ed2dff1174047c8a8d5c07182833969a8e25c2baf576f"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.10/steamship-0.9.10-x86_64-apple-darwin.tar.gz"
      sha256 "ec8494e1085a66e314949b7645cfce679ae9617aca31837276f42bbeabeef076"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.10/steamship-0.9.10-x86_64-linux-musl.tar.gz"
      sha256 "8c0d34275693066d34d6044d39b2c4be426a70834a957aa30d45fee0bda20b4b"
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
