# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.64/steamship-0.9.64-aarch64-apple-darwin.tar.gz"
      sha256 "1d3cccad025eae37456541852c7f85c8e98dc8e557393d84d423253c6cabc6ad"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.64/steamship-0.9.64-x86_64-apple-darwin.tar.gz"
      sha256 "24808d75dfb0facddb6a92e4ec352d08968e7ebdbef771467137ad2c6ef0b212"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.64/steamship-0.9.64-x86_64-linux-musl.tar.gz"
      sha256 "56158601454ec2871c52a235e39246c140f9e31bb1c8d51ba7259edb7cbcffb5"
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
