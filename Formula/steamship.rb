# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.39/steamship-0.9.39-aarch64-apple-darwin.tar.gz"
      sha256 "fd20d98f8acc2cd75b6b8a3688425735229dd957f9e77f0a48cd7c9fe19e7a4b"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.39/steamship-0.9.39-x86_64-apple-darwin.tar.gz"
      sha256 "8bcd66a376112145e653f15844edb0fd6da195aa2b5a0fcd8747be4c78012058"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.39/steamship-0.9.39-x86_64-linux-musl.tar.gz"
      sha256 "54146dbc4beef7df636db78cd61cfebda787a0bc09cb244ba9dba4b636ed8c3a"
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
