# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.1/steamship-0.8.1-aarch64-apple-darwin.tar.gz"
      sha256 "1109212e674666678599e720baa8311658491393d28642e15ca672980661e0c3"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.1/steamship-0.8.1-x86_64-apple-darwin.tar.gz"
      sha256 "3f93d4fd09ca0b3db580994af0b4bb38ab7c56d75e34e13167d3893144242b70"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.8.1/steamship-0.8.1-x86_64-linux-musl.tar.gz"
      sha256 "dabac98d7761a42abd59f8001eca56b0242408419b5c2e4b5b6ce26cd6b98799"
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
