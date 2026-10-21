# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.49/steamship-0.9.49-aarch64-apple-darwin.tar.gz"
      sha256 "c8bd3541c52a2d7d3571049deafda3491e9de9270d1964520330199f5348dabe"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.49/steamship-0.9.49-x86_64-apple-darwin.tar.gz"
      sha256 "374db5cad6fa2cc8e24c89ef996b6a4e69512011492ea56ac08fa5f2f4c43a4e"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.49/steamship-0.9.49-x86_64-linux-musl.tar.gz"
      sha256 "98cab9e2c59fc2b385d5bea967fcbff4997c8735d5e04a28507f324d2a7fdc9c"
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
