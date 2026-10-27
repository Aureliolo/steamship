# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.74/steamship-0.9.74-aarch64-apple-darwin.tar.gz"
      sha256 "180b6c79429f0f5e71f378000c0e8c4b6d1d67a311314edc08520e47091f4bca"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.74/steamship-0.9.74-x86_64-apple-darwin.tar.gz"
      sha256 "84138a503ee280e793654bd7c86b15699567143d7d66555a226b60e7e6ccfad2"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.74/steamship-0.9.74-x86_64-linux-musl.tar.gz"
      sha256 "11236e98732ffdc7da859f9f04a26e3d3f5473308daf4accc4e64c50f5e447ed"
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
