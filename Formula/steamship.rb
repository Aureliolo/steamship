# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.52/steamship-0.9.52-aarch64-apple-darwin.tar.gz"
      sha256 "f2a35848b77ea45a01082a40c0cf2289586e77470d6ff90a41a5afbe988a78ca"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.52/steamship-0.9.52-x86_64-apple-darwin.tar.gz"
      sha256 "03d638679c022c11e122fbc7eb7f49aad4c834f86583dfbbb8bae97f53cba08a"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.52/steamship-0.9.52-x86_64-linux-musl.tar.gz"
      sha256 "ad3074fa4e968a8f4c9cad3d9102ba42743c874b104693444b4773085df6b560"
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
