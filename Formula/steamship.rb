# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.96/steamship-0.9.96-aarch64-apple-darwin.tar.gz"
      sha256 "96813dcc01b4e14b07b20f3f3ac7168e6ff3c13f0e49447f2d341c632d5d1ca9"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.96/steamship-0.9.96-x86_64-apple-darwin.tar.gz"
      sha256 "cf82bb91ae9ec95f1ccf41314efd24f11b1674c879e562efd5b966b4777bdcaa"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.96/steamship-0.9.96-x86_64-linux-musl.tar.gz"
      sha256 "0b3a38dd8c6a3103e80fe8d051c138ee81e087133c4d79103de0bf7ea0cf699c"
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
