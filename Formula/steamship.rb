# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.77/steamship-0.9.77-aarch64-apple-darwin.tar.gz"
      sha256 "56553339d60e6b4671f3dae0fc3b445e87fa3be68b30274303eda9247c34288d"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.77/steamship-0.9.77-x86_64-apple-darwin.tar.gz"
      sha256 "96c390a93a8b309af11cdbbd1fe3fbea13376023bae72400ff2b8df5457a61b3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.77/steamship-0.9.77-x86_64-linux-musl.tar.gz"
      sha256 "b63520806a8642f140d1709998746f528497cc8ce476acca5dfeff50575d8a6d"
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
