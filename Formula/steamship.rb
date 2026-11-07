# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.91/steamship-0.9.91-aarch64-apple-darwin.tar.gz"
      sha256 "5b17c429de86859bd20ebc2e821637df076bf6ec43662d991dd3c6eb162b8d11"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.91/steamship-0.9.91-x86_64-apple-darwin.tar.gz"
      sha256 "cd66c5cfc1118766cc14b893b14b34a8f07decb521c809f1659a62e15c0efabc"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.91/steamship-0.9.91-x86_64-linux-musl.tar.gz"
      sha256 "513dc5e5640fe13468da556a41dd103fc1375c2cdb8a7280b516a4574de0ed0e"
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
