# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.47/steamship-0.9.47-aarch64-apple-darwin.tar.gz"
      sha256 "13d21fac925627a28b6e4506a0d70ccfd3178a397de8ebf7ca99dc72ca8188bb"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.47/steamship-0.9.47-x86_64-apple-darwin.tar.gz"
      sha256 "c4225f64b75de9b4a105d4d672f3d25e192faba88686a824017273c38ab8e280"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.47/steamship-0.9.47-x86_64-linux-musl.tar.gz"
      sha256 "d851efffa163913fb915540ac03efe5bdafcb46bf02c88a1e365985c9caaa4c0"
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
