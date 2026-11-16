# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.4/steamship-1.0.4-aarch64-apple-darwin.tar.gz"
      sha256 "adb7af83f550e9067d71c2b0925f9420bdead8127b9e65688f48337da495ae6c"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.4/steamship-1.0.4-x86_64-apple-darwin.tar.gz"
      sha256 "d38a6a2e5b6b9c03dc59227088856860cfe2b5c0d1596f14a6ac5352cd704957"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.4/steamship-1.0.4-x86_64-linux-musl.tar.gz"
      sha256 "8fccd93605366514480c1d1131fd071aa549575771497b37688fe9d7bb9f3ff7"
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
