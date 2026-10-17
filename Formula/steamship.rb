# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.18/steamship-0.9.18-aarch64-apple-darwin.tar.gz"
      sha256 "f6898f1fe2a9113a1052ceb28ed63d86b49b4b2f3f7de74796ecd10f06183e1a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.18/steamship-0.9.18-x86_64-apple-darwin.tar.gz"
      sha256 "d5c26208d15ccea189b220e16da6d34d0412f9945865d6a1f524ebc9d1a71a50"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.18/steamship-0.9.18-x86_64-linux-musl.tar.gz"
      sha256 "c62c50da30ebc4d447d562ee3b309f861491e3c2db2f229e8cc845daba0ebc20"
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
