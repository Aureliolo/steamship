# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.36/steamship-0.9.36-aarch64-apple-darwin.tar.gz"
      sha256 "cf3cdbd412655be85f715ff848c77b32ab6db8d1f03f1d029a405698dd2e739b"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.36/steamship-0.9.36-x86_64-apple-darwin.tar.gz"
      sha256 "9976983eeb78b1b80a85ecc12c9f83baec6e48aaeca23227f7cf80f48cc09e0c"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.36/steamship-0.9.36-x86_64-linux-musl.tar.gz"
      sha256 "890b30c21264c70043acd8202b289f311f638501688278000aedb224ecead383"
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
