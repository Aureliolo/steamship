# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.45/steamship-0.9.45-aarch64-apple-darwin.tar.gz"
      sha256 "415e729263ec2b610828b7e54d971191ce53647ccfaff6c13e678f5aaac03d87"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.45/steamship-0.9.45-x86_64-apple-darwin.tar.gz"
      sha256 "02705a0277be0f43fed2205f01e13b48afffc6168fbbc165d54e3ecc67b1a6d0"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.45/steamship-0.9.45-x86_64-linux-musl.tar.gz"
      sha256 "60b212f6c39a5bd8eb8fb75c84fcea812b8928a41002c4656173f619e6438c32"
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
