# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.7.1/steamship-0.7.1-aarch64-apple-darwin.tar.gz"
      sha256 "7592b8de32f430410ad7459380738a40915221f8cd33c4d5767168cddaae29eb"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.7.1/steamship-0.7.1-x86_64-apple-darwin.tar.gz"
      sha256 "5c664125a8478fc3ec17e61a50cd2570d5a4851efdd986ac3212f826cf0eb205"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.7.1/steamship-0.7.1-x86_64-linux-musl.tar.gz"
      sha256 "5828ae861015dfbb3a7476e81e0e8fe31dd2736cb2c859bbec8b812149d457b7"
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
