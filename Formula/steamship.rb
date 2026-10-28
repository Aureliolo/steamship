# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.75/steamship-0.9.75-aarch64-apple-darwin.tar.gz"
      sha256 "90f960282f76bd086e9a7140e5991acd9456f1ba68cae3b390226cbd32abee31"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.75/steamship-0.9.75-x86_64-apple-darwin.tar.gz"
      sha256 "94e8457e62968181a61951b2d9358a187872804aa01b4d4d5a0dd9cb4126c207"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.75/steamship-0.9.75-x86_64-linux-musl.tar.gz"
      sha256 "90cb726a00f1932b526a803e021c9729ab98f75cd36a19d3d52ee1af12a009d5"
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
