# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.1/steamship-1.0.1-aarch64-apple-darwin.tar.gz"
      sha256 "cec7bf85617f9d8cc1f3e0338bfeb189d8a762f01c6e80c0477bcce684473980"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.1/steamship-1.0.1-x86_64-apple-darwin.tar.gz"
      sha256 "8f79f86264bf975120ff9beacde8e91661b8f770c19ee2967992e639039c92ce"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.1/steamship-1.0.1-x86_64-linux-musl.tar.gz"
      sha256 "4d1a8085221d8856b00c77526adefc7a301271cf78ed63d8d26bf3205c6e2b25"
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
