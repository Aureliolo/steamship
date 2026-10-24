# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.62/steamship-0.9.62-aarch64-apple-darwin.tar.gz"
      sha256 "7032cad8c1a39215bddda13f5df04921ea8e1dd1f6a68d4a3345ac7a3d4bb4fb"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.62/steamship-0.9.62-x86_64-apple-darwin.tar.gz"
      sha256 "426a9a9eb5d479d48e19dea052be00b3f2c3727651757de6b07023cbd5ba6afb"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.62/steamship-0.9.62-x86_64-linux-musl.tar.gz"
      sha256 "07c2df88bd08286506401f4c431be207798b49b511956526d706e0b057af564b"
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
