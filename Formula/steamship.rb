# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.65/steamship-0.9.65-aarch64-apple-darwin.tar.gz"
      sha256 "0dd459443478df61d3912ab294ffaaee838a9323f81e01f804564ac94b7ae035"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.65/steamship-0.9.65-x86_64-apple-darwin.tar.gz"
      sha256 "fbb132ab3acdea0fd854e9ed5fc6d843a2f6336fb7e988ef22fe69ce4ca3bc0f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.65/steamship-0.9.65-x86_64-linux-musl.tar.gz"
      sha256 "40d144db74a98ff361a6b9f3a5b17b719df517379817c689bc825052413af30d"
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
