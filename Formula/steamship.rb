# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.95/steamship-0.9.95-aarch64-apple-darwin.tar.gz"
      sha256 "b7f9bc4309d0acb913b012a3a828bca63d2a4f682ec3a66cf4e1206207178ff0"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.95/steamship-0.9.95-x86_64-apple-darwin.tar.gz"
      sha256 "9b5dfa755b7b9399bc8b15f3556182c158b9b7e5d7cbc9daf85ddb34f611ad81"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.95/steamship-0.9.95-x86_64-linux-musl.tar.gz"
      sha256 "16a5c8837eb0233aec34e72b7edc67f5f4a8cb2a8cf907b2ce4ac0cf35925f10"
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
