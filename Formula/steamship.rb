# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.59/steamship-0.9.59-aarch64-apple-darwin.tar.gz"
      sha256 "dfe16c262fec4006d4bb30f28980be0950d8079d4c4ff861a00ff3abf2905d4a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.59/steamship-0.9.59-x86_64-apple-darwin.tar.gz"
      sha256 "3b06ec0cafe8dd50a1ab0118e59bab4f915dbc1b3f7e869f9763948087d85181"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.59/steamship-0.9.59-x86_64-linux-musl.tar.gz"
      sha256 "cad79fbdb363f592c423f3c48ae9e14c7c62a1a8f81bccbcac7eb339eb23f8c3"
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
