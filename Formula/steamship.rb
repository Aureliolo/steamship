# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.24/steamship-0.9.24-aarch64-apple-darwin.tar.gz"
      sha256 "dd12ed9e0fbf105c0303a670da73c4cc3682a78130eb02377436021db3686e3e"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.24/steamship-0.9.24-x86_64-apple-darwin.tar.gz"
      sha256 "e74d5b65599fe3a3543d9436f8a3805827529f0c079c54b90264271f98e28be1"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.24/steamship-0.9.24-x86_64-linux-musl.tar.gz"
      sha256 "7e11851e5b169bf2c5741cddb1976747afc96a58f888aff17e32a97edfe97391"
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
