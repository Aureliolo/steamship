# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.63/steamship-0.9.63-aarch64-apple-darwin.tar.gz"
      sha256 "79323a904a1e3c34a9e52cc754fc6f210e5ce4ac75e4a0d0af5849af361dcf74"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.63/steamship-0.9.63-x86_64-apple-darwin.tar.gz"
      sha256 "aab757ee96acaa72c970c6b8975832eabe2b54b8c6deab6213712e7853dde3b0"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.63/steamship-0.9.63-x86_64-linux-musl.tar.gz"
      sha256 "80f1f17fe92059dccb2856dd7bc0becfba5e0778dcd58abdac59af2f4d81329e"
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
