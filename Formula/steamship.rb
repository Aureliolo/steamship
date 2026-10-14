# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.6/steamship-0.9.6-aarch64-apple-darwin.tar.gz"
      sha256 "10473f927c2a7cb2debd3c8777c4086a65b2cffbd861409e32595e5a262c4452"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.6/steamship-0.9.6-x86_64-apple-darwin.tar.gz"
      sha256 "90e2cf3e8383a309dde04c8d4f5152bc4385af4c8af4f9b693059706d6c5df75"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.6/steamship-0.9.6-x86_64-linux-musl.tar.gz"
      sha256 "dfeda093763ec01dfc7af905dd22e1d222e3a6a88f1f0b8ede982aa723d406d4"
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
