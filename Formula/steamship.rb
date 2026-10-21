# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.50/steamship-0.9.50-aarch64-apple-darwin.tar.gz"
      sha256 "1aed91a31fe7d70b6bc43e591b9d292e1643f53e25e653d38b62d963321824db"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.50/steamship-0.9.50-x86_64-apple-darwin.tar.gz"
      sha256 "ec773e2ddde69b03c5e4c9f8cf88211c266a3259e9b666102da91cd562ef98b3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.50/steamship-0.9.50-x86_64-linux-musl.tar.gz"
      sha256 "b0463edc2c141bc330d10fbb9d93b1bdfe7f2061eac0e6a8828102b507470205"
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
