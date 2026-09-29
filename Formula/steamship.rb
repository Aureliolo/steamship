# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.3/steamship-0.6.3-aarch64-apple-darwin.tar.gz"
      sha256 "3f61b998037b1721bdcb43afd956432c7df855f6e1828ed07a552b34e7a861ec"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.3/steamship-0.6.3-x86_64-apple-darwin.tar.gz"
      sha256 "dc3553f2dcf1fda70d42f706fb35c20b98b4de468df6496e635b9765a9808716"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.3/steamship-0.6.3-x86_64-linux-musl.tar.gz"
      sha256 "32049bd2d6dd186fbd6c3d191750775557c64de5022ec3dd7b011e898fad46a1"
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
