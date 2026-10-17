# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.26/steamship-0.9.26-aarch64-apple-darwin.tar.gz"
      sha256 "483ffddbfcc1b90b8c25b90834904ebe5f88aa206edf3296a344e8a5b16a6fa4"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.26/steamship-0.9.26-x86_64-apple-darwin.tar.gz"
      sha256 "4e247ba9cce7e442f426b7cbcbbe4e785ab73721e64ce01783792b276c0a18d8"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.26/steamship-0.9.26-x86_64-linux-musl.tar.gz"
      sha256 "5f937ce973fc0e7ee70f7c2e32969ad18d886181b06ce6a5633959f8c24201d8"
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
