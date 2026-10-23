# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.60/steamship-0.9.60-aarch64-apple-darwin.tar.gz"
      sha256 "43ee094c4bb3cc8ddda93fe11aaab4ae8449315aa2426decc16d5c169647781c"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.60/steamship-0.9.60-x86_64-apple-darwin.tar.gz"
      sha256 "f1aa308b9a8544c8e548cb5fe58c9d6e3452ff224866728a338bcfffe13991a5"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.60/steamship-0.9.60-x86_64-linux-musl.tar.gz"
      sha256 "dd5e329c764a0c2221e9f26f857649d45bfd040b7635c37df254c5091e905371"
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
