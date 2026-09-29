# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.7.0/steamship-0.7.0-aarch64-apple-darwin.tar.gz"
      sha256 "3f6916ea79e73b98b712611cb8ed549bc949027ecd2f5a22b26c24452897e29c"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.7.0/steamship-0.7.0-x86_64-apple-darwin.tar.gz"
      sha256 "266059bb381944874ec0e4a25ed31b1ff3d8fd65a17bc7258cf99d36f80d107e"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.7.0/steamship-0.7.0-x86_64-linux-musl.tar.gz"
      sha256 "e3ce85dbd470c44658fa6c535cb7cfdc2f04d9ffb78fb73579bb3c631fc1de12"
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
