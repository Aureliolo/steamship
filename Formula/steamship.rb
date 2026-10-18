# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.28/steamship-0.9.28-aarch64-apple-darwin.tar.gz"
      sha256 "6a4823d88180b4bf78e9a11f9f7b104a413960d6d4e8665c8a9e8e86315c57ee"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.28/steamship-0.9.28-x86_64-apple-darwin.tar.gz"
      sha256 "12295ea645cd3f5346bb91a7446ae3ea2871046ffaaf8aca6981aa06ef921f46"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.28/steamship-0.9.28-x86_64-linux-musl.tar.gz"
      sha256 "320216213318bb7d9df4c7d0052270558a2fc1586d8848cb925f61c9b724544d"
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
