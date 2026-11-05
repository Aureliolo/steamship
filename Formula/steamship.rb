# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.88/steamship-0.9.88-aarch64-apple-darwin.tar.gz"
      sha256 "4f14d6899378e266a0ff54f11fa1c1668ef6324d4ccce46ad42e1f416272d1e6"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.88/steamship-0.9.88-x86_64-apple-darwin.tar.gz"
      sha256 "598c145291a78a97b5fc22e6ad8af13340f57fbb9ffc3bfc529fc6c6a5da1ebf"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.88/steamship-0.9.88-x86_64-linux-musl.tar.gz"
      sha256 "c6027491ee5c395780b231dea5f637bde6e66579dc852a2f339ec6ed0531df5b"
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
