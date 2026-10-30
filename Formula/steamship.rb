# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.78/steamship-0.9.78-aarch64-apple-darwin.tar.gz"
      sha256 "e6c1d6eb02c06345c76ebb80e2d1fb374d7c74f1e1cc35d722904be46fae37d5"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.78/steamship-0.9.78-x86_64-apple-darwin.tar.gz"
      sha256 "86af3acf3e4f583cdaf9feac7c2629f81127808abe37cba64d2623c4a43485b9"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.78/steamship-0.9.78-x86_64-linux-musl.tar.gz"
      sha256 "bef828ef0efba05bddd59bdf9ef65668d5de56027f678e2c02b7e608123c65f2"
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
