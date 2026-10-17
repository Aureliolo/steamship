# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.16/steamship-0.9.16-aarch64-apple-darwin.tar.gz"
      sha256 "9693804b4e7c580a8ff2c9606bc503cc88cea1fbbe56d4aa201bdfc6b39c4791"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.16/steamship-0.9.16-x86_64-apple-darwin.tar.gz"
      sha256 "e24c43f8b5aa398fa072baa798824229af864bed1ac34f56f6c6651044570e0f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.16/steamship-0.9.16-x86_64-linux-musl.tar.gz"
      sha256 "fa97ca5f6dbf444166ad66d9c6a47cce30285b1f8d620735087d5acfced19f53"
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
