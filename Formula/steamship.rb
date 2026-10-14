# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.7/steamship-0.9.7-aarch64-apple-darwin.tar.gz"
      sha256 "242d329bb6280ff1b6c06201a4d7b80bc989df60c656f0a43f81e1c6053cd971"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.7/steamship-0.9.7-x86_64-apple-darwin.tar.gz"
      sha256 "b98e518871819a8e4df7f057d7099709f386152d24380fdd7c1b7e0153678d30"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.7/steamship-0.9.7-x86_64-linux-musl.tar.gz"
      sha256 "9d360ce3a05c9833c01cd6c6db1e6e590d290a710320bb48709a18dbd6be398d"
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
