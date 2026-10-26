# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.70/steamship-0.9.70-aarch64-apple-darwin.tar.gz"
      sha256 "75dc9135eb5c0d5ba2d79d0389b18028ac18e99b8dde854745231d9809406134"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.70/steamship-0.9.70-x86_64-apple-darwin.tar.gz"
      sha256 "366efb557d7c1a89755003fbaf3f73c7888b7fe2c0b46f14ada7051f04efa387"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.70/steamship-0.9.70-x86_64-linux-musl.tar.gz"
      sha256 "dd9ae5d2d7746aff9ad365449a2491d785a9e355a8bc7039c02fd45375435d3d"
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
