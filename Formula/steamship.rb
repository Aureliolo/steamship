# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.85/steamship-0.9.85-aarch64-apple-darwin.tar.gz"
      sha256 "bb6a5e69893b7efb217246d838d4cb3963b82f4bcfb7a6c3cd61f9f98a6371dc"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.85/steamship-0.9.85-x86_64-apple-darwin.tar.gz"
      sha256 "32d9c5b128a50426ebbb917988552e12dc12a4d7b05d24d18f47828e86634368"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.85/steamship-0.9.85-x86_64-linux-musl.tar.gz"
      sha256 "bf20bd52949c3f460c01f27ad51be2fec7af9555ecbef490e3886d20007d1180"
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
