# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.4.0/steamship-0.4.0-aarch64-apple-darwin.tar.gz"
      sha256 "7c4dc364744a653d695532ae73147853d0e07c379998ec73bae978641d9b711a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.4.0/steamship-0.4.0-x86_64-apple-darwin.tar.gz"
      sha256 "8868cfd1dad85e906fdf239425670cf10bc4e3bd5bb2c065624da1e80d301390"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.4.0/steamship-0.4.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "acb000f7ba57c24c8a92b0906b95526a70246d3571ac5c0e611dd89e9b598faf"
    end
  end

  def install
    bin.install "steamship"
  end

  test do
    assert_equal "steamship #{version}", shell_output("#{bin}/steamship --version").strip
  end
end
