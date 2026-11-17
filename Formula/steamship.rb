# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.5/steamship-1.0.5-aarch64-apple-darwin.tar.gz"
      sha256 "832101963065800707af8904e0a0caf6c7eb8776bde0a3487ff9d6231645a634"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.5/steamship-1.0.5-x86_64-apple-darwin.tar.gz"
      sha256 "9616d371e50f64a510268b0b368f8c6ebb2f3bda6dc5e874e9f9438edbe5dd43"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.5/steamship-1.0.5-x86_64-linux-musl.tar.gz"
      sha256 "53e8de3f948d67ec9c9c438db50e45808de4020ab63aaea0c15c6740326d4068"
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
