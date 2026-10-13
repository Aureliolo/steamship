# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.2/steamship-0.9.2-aarch64-apple-darwin.tar.gz"
      sha256 "24d91082c1928ad748ef4eba08f7e3d68b97056a718f9473afd9c6ada625fefd"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.2/steamship-0.9.2-x86_64-apple-darwin.tar.gz"
      sha256 "342bf8f095ec377dfa047b4543b465d062a8db29e607b92a327303e5a37577e7"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.2/steamship-0.9.2-x86_64-linux-musl.tar.gz"
      sha256 "a4cab5e388180f37ec89b227ed35fd391c14deedcec0b351e39e5b9c994af243"
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
