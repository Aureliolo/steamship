# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.46/steamship-0.9.46-aarch64-apple-darwin.tar.gz"
      sha256 "a4dd7e4855f731526f0874cfcae1d4621278884eaff5e91d4ed003d1f35a012f"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.46/steamship-0.9.46-x86_64-apple-darwin.tar.gz"
      sha256 "87de23d0a35e16748bf3459e1a5198cf2c501e8f736385489c6700eeacfbb8d9"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.46/steamship-0.9.46-x86_64-linux-musl.tar.gz"
      sha256 "a32a4a90ae7bd274ab35843004c5c94e8ef6625df2a094b47ccdb231cdd8e022"
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
