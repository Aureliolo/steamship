# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.58/steamship-0.9.58-aarch64-apple-darwin.tar.gz"
      sha256 "92732897060b460e8883806022a0a11a0a855ff58a52eaeb7bbaf84a9c068954"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.58/steamship-0.9.58-x86_64-apple-darwin.tar.gz"
      sha256 "221dbb8df18003e127e95cdef19cbb7be4ba379a5b31c3b3bd6ab6d421e08b25"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.58/steamship-0.9.58-x86_64-linux-musl.tar.gz"
      sha256 "ebdcd49f9cd355021315999e213b85022ada8d12acb0522c88f68db6c4bd6214"
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
