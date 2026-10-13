# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.1/steamship-0.9.1-aarch64-apple-darwin.tar.gz"
      sha256 "e4d779c3a6ef4d588932394982b8bb24b2b058e97522acb9e399e1550a070d51"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.1/steamship-0.9.1-x86_64-apple-darwin.tar.gz"
      sha256 "3fe44c668d0368b8dfa70b6aea4cf773d85002c8e181dc3bd29edc6038c67d94"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.1/steamship-0.9.1-x86_64-linux-musl.tar.gz"
      sha256 "7fb40d6c5978e1af7c94f4f5db54296018ad4481fc182a8c1ba012f5702e3450"
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
