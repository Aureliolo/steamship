# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.61/steamship-0.9.61-aarch64-apple-darwin.tar.gz"
      sha256 "3c3efea49cd1e491a762ebbb452f24a2ae8c8748227d02d8efe5bcb64be36b27"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.61/steamship-0.9.61-x86_64-apple-darwin.tar.gz"
      sha256 "1e2ea56e22f855ff11e5a1b4f8b4a285d471e3d3ce3e9831eaf5ff884ee38a09"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.61/steamship-0.9.61-x86_64-linux-musl.tar.gz"
      sha256 "f17e7b98e890e090a4474a44d4c0a2f24ba6f825c6c1f56570f2e2cacada38e4"
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
