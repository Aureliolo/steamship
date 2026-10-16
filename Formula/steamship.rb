# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.12/steamship-0.9.12-aarch64-apple-darwin.tar.gz"
      sha256 "6784250a2b58bc745936a140453809613744437e568f52122ef5d6db6c1744b2"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.12/steamship-0.9.12-x86_64-apple-darwin.tar.gz"
      sha256 "a1a39a9ac59e3fcb98850cb20f87c0dbf0e5451f0e7a29881621fba6a7fed86c"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.12/steamship-0.9.12-x86_64-linux-musl.tar.gz"
      sha256 "ccf57e519f85f45c342bd35888f0922f396b97e72830503fe74dadee2729ab0c"
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
