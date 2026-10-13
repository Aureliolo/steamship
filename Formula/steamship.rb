# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.4/steamship-0.9.4-aarch64-apple-darwin.tar.gz"
      sha256 "3b26050d2d55034daf41863b2abe4643ff8232e884ca5af0546953988d3247fd"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.4/steamship-0.9.4-x86_64-apple-darwin.tar.gz"
      sha256 "4cd79aadfae0ad1316e48128425a9a95d8120f80513dae23d6b14f02da58d161"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.4/steamship-0.9.4-x86_64-linux-musl.tar.gz"
      sha256 "d421aaa5b49d17cdd17ce5acc1ad8f164d04db412ccc11bc749bf511536ea9bc"
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
