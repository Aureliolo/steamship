# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.76/steamship-0.9.76-aarch64-apple-darwin.tar.gz"
      sha256 "d2f911f9fcb119c0684b9d06835a5c5f4a14f5e838b6d5b335d2430b5cf8f4fc"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.76/steamship-0.9.76-x86_64-apple-darwin.tar.gz"
      sha256 "4fd12d740d7ffc19044e7e524d525841e09572655374f9db6e0e2c76f8362ca2"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.76/steamship-0.9.76-x86_64-linux-musl.tar.gz"
      sha256 "f6b2cb3814327e5b6fcbe61eb2450990639ee2a8891599d71f4a48d1e1f320d0"
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
