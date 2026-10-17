# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.19/steamship-0.9.19-aarch64-apple-darwin.tar.gz"
      sha256 "1ec48ddcd88e052d8ef32cbe018364678dbbd258600d05b3d510f0db1dd1610e"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.19/steamship-0.9.19-x86_64-apple-darwin.tar.gz"
      sha256 "6ccffc4edf744100ed5e7ba812f759cb9efa15064c6c6ec1f73933b07360eda1"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.19/steamship-0.9.19-x86_64-linux-musl.tar.gz"
      sha256 "eee343176e4c5825c47083fd9a652572245d2aaeb1cb937c9c2354ae5528a80a"
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
