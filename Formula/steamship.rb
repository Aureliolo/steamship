# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.56/steamship-0.9.56-aarch64-apple-darwin.tar.gz"
      sha256 "e9bd4914018671f45eb39ff2f2cdd63df3e9078dacc0120f8d932dfd598594ee"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.56/steamship-0.9.56-x86_64-apple-darwin.tar.gz"
      sha256 "5ea0b060a64765552b68436d4382ff6a93a913b11c7ab9283452f950af90aeb3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.56/steamship-0.9.56-x86_64-linux-musl.tar.gz"
      sha256 "e9537e6bd39273000081ec24d47858f09f84df6f28b9828f9893245d054f25cf"
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
