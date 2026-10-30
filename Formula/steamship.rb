# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.80/steamship-0.9.80-aarch64-apple-darwin.tar.gz"
      sha256 "6d2c847193c5f1b13962464430a8f80b2668d53f85216ccd5eb9e52554923079"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.80/steamship-0.9.80-x86_64-apple-darwin.tar.gz"
      sha256 "f78f8f942cf04dfe199cccfdcd24d753436efdc462caed62b9ab2529d1b696a2"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.80/steamship-0.9.80-x86_64-linux-musl.tar.gz"
      sha256 "f0c0c740d5444a7b28678ac6cfac37423ace4add4ef2b95406f02303726957a4"
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
