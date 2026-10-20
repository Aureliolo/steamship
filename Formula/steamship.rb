# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.41/steamship-0.9.41-aarch64-apple-darwin.tar.gz"
      sha256 "539ba14df8e51eced1cd8a5226bcd2e9eb182bdc44d304105e7bcdfdcd9f706a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.41/steamship-0.9.41-x86_64-apple-darwin.tar.gz"
      sha256 "2a5551c82734f7800049bae6b67f3d72409e35c3da222d86f97acdb1815f89c2"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.41/steamship-0.9.41-x86_64-linux-musl.tar.gz"
      sha256 "f16ae5a46c3400a1e3e27acfd4f0df365458c5350ebd9aa9581d418f016b439c"
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
