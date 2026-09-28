# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.0/steamship-0.6.0-aarch64-apple-darwin.tar.gz"
      sha256 "97a781efcf713f716ec11710d93deffb711a9586242fefd167bde5c656fbdfbf"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.0/steamship-0.6.0-x86_64-apple-darwin.tar.gz"
      sha256 "f01e94a85b3f9456ee4966733f32dc90c0fa23da340eb35a8e81beb7f33e7374"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.6.0/steamship-0.6.0-x86_64-linux-musl.tar.gz"
      sha256 "c75065664c38e34a7cc8e1bb92d0bf06f954f6b7f74a38d3ce6ea6380c9e953d"
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
