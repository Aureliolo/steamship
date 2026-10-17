# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.15/steamship-0.9.15-aarch64-apple-darwin.tar.gz"
      sha256 "e5d318371bfbad165529f3fbb2f7002f6c09cbe231aaa66fc44aa84912976af6"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.15/steamship-0.9.15-x86_64-apple-darwin.tar.gz"
      sha256 "f746c1426bac0e24f0058892dc62e57830043285625703dcb820bff459f68c49"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.15/steamship-0.9.15-x86_64-linux-musl.tar.gz"
      sha256 "36de9d70661350b8e95a18030c477e7107dd9f96474dedf002467f1626c3c628"
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
