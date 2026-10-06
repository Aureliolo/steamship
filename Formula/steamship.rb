# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.2/steamship-0.8.2-aarch64-apple-darwin.tar.gz"
      sha256 "af70a7ece99d9ab6bcba0bdbd5c2d2ecb11bb9bd0b96f71fde085edd0c1b1c83"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.2/steamship-0.8.2-x86_64-apple-darwin.tar.gz"
      sha256 "3f6510f2c7cc31e6998004002fb8fc100ba5846c4da4b55adbcb0972f1b18f81"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.2/steamship-0.8.2-x86_64-linux-musl.tar.gz"
      sha256 "51a2b2d3da7ec88d13694d851980445ede28ced86ccfba1dfdf3d4807e4aec8a"
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
