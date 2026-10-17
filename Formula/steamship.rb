# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.17/steamship-0.9.17-aarch64-apple-darwin.tar.gz"
      sha256 "8f17ddfed35ba16cc71a90d0c0b1e5626545145ae8204c46fe1e1da419c090db"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.17/steamship-0.9.17-x86_64-apple-darwin.tar.gz"
      sha256 "4390a2791da6b735c5d87cd6cfd7cc1c7810a156d152f632efca48ed3bd7bc02"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.17/steamship-0.9.17-x86_64-linux-musl.tar.gz"
      sha256 "a8d5eaa2bf49b5cfa163e5c47dfce4e4029f0ca500f64d59141f034e96b1d97c"
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
