# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.5.1/steamship-0.5.1-aarch64-apple-darwin.tar.gz"
      sha256 "760c1fdd939d30b3ae4c36e6eb4a642db1856823241b3895178f315eb9b9330e"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.5.1/steamship-0.5.1-x86_64-apple-darwin.tar.gz"
      sha256 "3c7770ad9c60b075dba655c4d0303e2a44d1d8c3122e6903c496e486d6050f1f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.5.1/steamship-0.5.1-x86_64-linux-musl.tar.gz"
      sha256 "ff286acc62d9ece89ecde2415ed2ec951f7301e7bea37275780518f7e80e1b08"
    end
  end

  def install
    bin.install "steamship"
  end

  test do
    assert_equal "steamship #{version}", shell_output("#{bin}/steamship --version").strip
  end
end
