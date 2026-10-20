# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.42/steamship-0.9.42-aarch64-apple-darwin.tar.gz"
      sha256 "6b9330ec0a652f102ef0c68949f083a6f723ad508cfde4b36582b2022d55edfb"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.42/steamship-0.9.42-x86_64-apple-darwin.tar.gz"
      sha256 "d40d39bed09dca720090ede30aaa1d4a9395c68e5e780e05b93dac9f162e2354"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.42/steamship-0.9.42-x86_64-linux-musl.tar.gz"
      sha256 "dfbee4b819e48b8dca244a9d69ad031a034dd6aa31f283f9472f3d397c2c9551"
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
