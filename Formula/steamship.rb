# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.7/steamship-1.0.7-aarch64-apple-darwin.tar.gz"
      sha256 "9a6e5fb947a4a41a0f7e1327253ef9ae653b1386a27e05e21a1a504e8c51302f"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.7/steamship-1.0.7-x86_64-apple-darwin.tar.gz"
      sha256 "2901d0074976ca494e13f3d79e76d5c90e57147b9991e38781702cba21257b22"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v1.0.7/steamship-1.0.7-x86_64-linux-musl.tar.gz"
      sha256 "37cc7ea7e529000792f9eaf89a5bf48720211a1f9ee5db6541bf29929448c511"
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
