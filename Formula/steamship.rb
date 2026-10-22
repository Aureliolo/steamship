# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.53/steamship-0.9.53-aarch64-apple-darwin.tar.gz"
      sha256 "b9d5410a823b475b22990c481eb653de8f5030d8490eb2423d30b9879fc1e352"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.53/steamship-0.9.53-x86_64-apple-darwin.tar.gz"
      sha256 "097b07de5342685a071dadd7efddb5b7742867544fcfccdaed81d77366d4d900"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.53/steamship-0.9.53-x86_64-linux-musl.tar.gz"
      sha256 "a0f934b81a7f04c19a81c2c2d9469b5e705bc15dacc58f360de547aef852b3ab"
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
