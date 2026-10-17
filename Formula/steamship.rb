# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.23/steamship-0.9.23-aarch64-apple-darwin.tar.gz"
      sha256 "ca168882abe7d4a273bc5bb3c52ca1a760a8877eebf54d7e289bf8b5b39094a7"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.23/steamship-0.9.23-x86_64-apple-darwin.tar.gz"
      sha256 "3e5ef0649e25a8b4e8a492bf5a3be919b94eff6b22b0291eda9bb6ce5e01f520"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.23/steamship-0.9.23-x86_64-linux-musl.tar.gz"
      sha256 "fdcd6b801e3222ef2f40500e1a4063227deda69cd386229bd93ea33cb123d6bb"
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
