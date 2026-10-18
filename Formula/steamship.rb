# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.33/steamship-0.9.33-aarch64-apple-darwin.tar.gz"
      sha256 "c8767aefd6fa564e0677e4858f3b1e3ecdb5f16e5d195c8db6a4fd3610da0b50"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.33/steamship-0.9.33-x86_64-apple-darwin.tar.gz"
      sha256 "7e754c6c44acdbda4f0ce7f433fd1d7eae4e1acebdb999997147348abfe8231d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.33/steamship-0.9.33-x86_64-linux-musl.tar.gz"
      sha256 "74d35667165530909797204eb889da42739138838179be7978e9110ff606454a"
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
