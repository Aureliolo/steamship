# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.94/steamship-0.9.94-aarch64-apple-darwin.tar.gz"
      sha256 "a1a20513b80fde94be1ed509de33b26c57230ca7b2f0ed1e411c1eca6a287f17"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.94/steamship-0.9.94-x86_64-apple-darwin.tar.gz"
      sha256 "a2d27864402e9d17ae1d280107d4f4478495f540ec29ff7711f755efb09c79c9"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.94/steamship-0.9.94-x86_64-linux-musl.tar.gz"
      sha256 "0252c8c301c18ef70fa428a78d72852fcf30421a26a5d08d3f07031e7d4e1f4a"
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
