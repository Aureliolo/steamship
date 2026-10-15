# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.11/steamship-0.9.11-aarch64-apple-darwin.tar.gz"
      sha256 "ff20f7dce9efdc106e9370c0a05721b4a58bbb0cbc153e144d79eb2c49f96576"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.11/steamship-0.9.11-x86_64-apple-darwin.tar.gz"
      sha256 "e876e227220673e166e7d8226ae67ddde7e6977a2ee5221249307206ebe652c3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.11/steamship-0.9.11-x86_64-linux-musl.tar.gz"
      sha256 "a65b388c953f5cecf75236b3725499e90e554ed9d59a344cef4ca7e6b7cc36af"
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
