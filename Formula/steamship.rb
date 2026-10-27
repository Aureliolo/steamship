# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.72/steamship-0.9.72-aarch64-apple-darwin.tar.gz"
      sha256 "40757c86e1f978b1f15049898de95e27b9963898ab9dfe3d07ba64798f754a72"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.72/steamship-0.9.72-x86_64-apple-darwin.tar.gz"
      sha256 "47a5e887e81fe9d63176985e5145eab1c9b5ea7518f24d781c62ce299103f06d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.72/steamship-0.9.72-x86_64-linux-musl.tar.gz"
      sha256 "ffe4e4b1cc768ab4d6bd56a9aa8a6c23f6051458418f0eaf69acc5139683ef2d"
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
