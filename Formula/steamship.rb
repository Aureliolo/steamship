# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.48/steamship-0.9.48-aarch64-apple-darwin.tar.gz"
      sha256 "2e561e88972a1004be9e5250c00f4397a00a0d0bce116134116b74f4084cee23"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.48/steamship-0.9.48-x86_64-apple-darwin.tar.gz"
      sha256 "aa4e3fb9039c98bcbf8ed450b76c1641e5dd291d0712c300c902fcc6e6cc5a84"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.48/steamship-0.9.48-x86_64-linux-musl.tar.gz"
      sha256 "48d517e85bca02be099808335712cf37d49518eb2368a6409731d584ad22e11c"
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
