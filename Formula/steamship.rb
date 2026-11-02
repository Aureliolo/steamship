# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.86/steamship-0.9.86-aarch64-apple-darwin.tar.gz"
      sha256 "acb7208877e515d59512c9960b56500f4af703f6f3be0e3124b9c2dd0f0d138d"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.86/steamship-0.9.86-x86_64-apple-darwin.tar.gz"
      sha256 "572afac64d6f392f22cf4dfa9c5228599ff3f0879585f7ca43f2bbf7512a1ef2"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.86/steamship-0.9.86-x86_64-linux-musl.tar.gz"
      sha256 "d2819d3bac79f47a899ac273969dae8ddff43e9fda05233f57fab2b617f1722d"
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
