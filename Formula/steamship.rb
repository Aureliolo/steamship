# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.2/steamship-1.0.2-aarch64-apple-darwin.tar.gz"
      sha256 "066bbba1b3a3070eaeadc40e73913917a56e2169541bbeb1db54bda25777aa44"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.2/steamship-1.0.2-x86_64-apple-darwin.tar.gz"
      sha256 "1c3e0041df8b336c02d6ff8fba99f0d4acc7277ca70e8e38fd5291ee438090aa"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.2/steamship-1.0.2-x86_64-linux-musl.tar.gz"
      sha256 "dcecc9e4af2794671d3eb2a832b20a4b7c49c6f8fe582452b713a97c64fc4b64"
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
