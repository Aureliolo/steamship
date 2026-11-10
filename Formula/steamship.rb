# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.93/steamship-0.9.93-aarch64-apple-darwin.tar.gz"
      sha256 "6492afc951995a53f3dae632d0f54e9ad64ea30700a5cd916c56724b7fbbed6a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.93/steamship-0.9.93-x86_64-apple-darwin.tar.gz"
      sha256 "c5cd3f4b1d0d85f95b3f33da55b76cc2e571134f25a8ca1bbbb1cc4e2837c0cb"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.93/steamship-0.9.93-x86_64-linux-musl.tar.gz"
      sha256 "ade13132cb666f0685399dd6cd5a1cd42c899ebaa542619081492d84479be8f9"
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
