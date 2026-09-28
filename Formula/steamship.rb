# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.2/steamship-0.6.2-aarch64-apple-darwin.tar.gz"
      sha256 "c269fe53800a56e9b584ae24cc6653217ce3f41cca392a5fe87a171c5808de22"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.2/steamship-0.6.2-x86_64-apple-darwin.tar.gz"
      sha256 "7be10d0c85a330cc8de73f64d353a3bb65f00879dd50faea4662368db01190e8"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.2/steamship-0.6.2-x86_64-linux-musl.tar.gz"
      sha256 "c4461aaccaaa09eba99891cf7264e6edd66563ec1c72c638e65e46e0d78da7bb"
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
