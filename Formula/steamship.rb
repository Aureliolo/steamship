# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.73/steamship-0.9.73-aarch64-apple-darwin.tar.gz"
      sha256 "4b51fe91c28926404b1c0b4950972f75b116508520cd0b5f78d928f647a50a2a"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.73/steamship-0.9.73-x86_64-apple-darwin.tar.gz"
      sha256 "3868ad5493e2e7be6a0a2e093ca26fb41fae29fe987300b54097c563047ab78d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.73/steamship-0.9.73-x86_64-linux-musl.tar.gz"
      sha256 "28eda7ec148630502dffe339f34b13a2c31b2d3b4cb18122f904b4d3253f4dd7"
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
