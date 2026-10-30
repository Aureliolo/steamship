# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.82/steamship-0.9.82-aarch64-apple-darwin.tar.gz"
      sha256 "f3ec878daf86abf0023a04b2f7b1e67df580c54eaa346c8498c0a76d492451de"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.82/steamship-0.9.82-x86_64-apple-darwin.tar.gz"
      sha256 "78a224399495e9513fd28219ab9e353c555b262bd9fbd3dd48d0104102bd82d0"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.82/steamship-0.9.82-x86_64-linux-musl.tar.gz"
      sha256 "ebb2f5785b557485823e6511e914a98ba99baa41a687af503e0cac1a46dcc5ea"
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
