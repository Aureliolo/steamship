# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.0/steamship-1.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "63271e5479fa41cf45106bbba3fa53325cdd26659202a6b7624c9c58daf5262d"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.0/steamship-1.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "cbb90e2245f0905632e3273c6c48b209f534d2a9403cf7a093ee36e1092c4946"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.0/steamship-1.0.0-x86_64-linux-musl.tar.gz"
      sha256 "e3e096aad4835811b7763208b98d36ac3d456ea6bfaac1e1a852afdfb5884eb3"
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
