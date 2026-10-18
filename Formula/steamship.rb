# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "https://github.com/Aureliolo/steamship"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.31/steamship-0.9.31-aarch64-apple-darwin.tar.gz"
      sha256 "48f97a7583e6b1fa4525f1253c014cefa9db70714289de012674c5a07d74743d"
    end
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.31/steamship-0.9.31-x86_64-apple-darwin.tar.gz"
      sha256 "1399868aa813dcf470ba7c069a9510ae37450436f1021324b095fc1530bab953"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Aureliolo/steamship/releases/download/v0.9.31/steamship-0.9.31-x86_64-linux-musl.tar.gz"
      sha256 "2f3cb75a45952d42ebeff0a50aa8052722a1177483647b58dd9be0c4ca91d6ee"
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
