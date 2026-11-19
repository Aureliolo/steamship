# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.6/steamship-1.0.6-aarch64-apple-darwin.tar.gz"
      sha256 "ca4c56d2a5f920307e070edf27e5c5211755c222af157a348f0abb4a4e141d85"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.6/steamship-1.0.6-x86_64-apple-darwin.tar.gz"
      sha256 "e53fad1b7455e40e37ed813a1b7b86e2d88aefeebd3f124e9036de5e7c6a2000"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.6/steamship-1.0.6-x86_64-linux-musl.tar.gz"
      sha256 "289320168a49c13ff920052ade1d5dc0365e77dfc21268a4ccb36ecd557ae3d5"
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
