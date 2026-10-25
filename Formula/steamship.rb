# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.67/steamship-0.9.67-aarch64-apple-darwin.tar.gz"
      sha256 "8f4101b47f684bc10b319c49249065644b0e61f9e3b991ecb011719fa499d78b"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.67/steamship-0.9.67-x86_64-apple-darwin.tar.gz"
      sha256 "5b76a0b9761985c233f9642d3c6e857b377e57425e26e25a7e30edc79182d262"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.67/steamship-0.9.67-x86_64-linux-musl.tar.gz"
      sha256 "77a8da9f02184fe9281e130773e1cc503fa4dd2bd763f70c65bb4c6351d5570c"
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
