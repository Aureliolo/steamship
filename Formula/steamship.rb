# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.25/steamship-0.9.25-aarch64-apple-darwin.tar.gz"
      sha256 "a02d87b32671615cfd1b6d10f90e1930bb97320c238d6fb383de1b48feaa9c01"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.25/steamship-0.9.25-x86_64-apple-darwin.tar.gz"
      sha256 "04dfcf25183d6c3ddb31e0833276b47c53027076cd511894fda4629283f12e38"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.25/steamship-0.9.25-x86_64-linux-musl.tar.gz"
      sha256 "b995dc4ee85254438258d6da210a1428e80b6e3740db73171b44567726cb582e"
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
