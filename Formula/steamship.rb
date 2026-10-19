# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.38/steamship-0.9.38-aarch64-apple-darwin.tar.gz"
      sha256 "acb97c6aa33a635715e5ce268818da4a07a835d4272dafc2cf087fd419aaf6cf"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.38/steamship-0.9.38-x86_64-apple-darwin.tar.gz"
      sha256 "1385f334f8a6ce3ebc42954c45cc16320327193f3f67b94e11ccc43ad639a0f4"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.38/steamship-0.9.38-x86_64-linux-musl.tar.gz"
      sha256 "db882cc6a75914c9d4e45c01b5c52b034e51df66a499be1042c2fdb32edae434"
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
