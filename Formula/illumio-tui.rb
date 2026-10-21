# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.2"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.2/illumio-tui-0.5.2-aarch64-apple-darwin.tar.gz"
      sha256 "ebd13319a01fb8d695545e5cbb8b69e1b703145e07189b89a61c16ac2cd95ff4"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.2/illumio-tui-0.5.2-x86_64-apple-darwin.tar.gz"
      sha256 "472a4c3dcd4e27be1118c8fa12ee3a24a959b4ca6379bbda44aea7b7ced12452"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.2/illumio-tui-0.5.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "c8861d8c54f0073981c4fa215ab8ba8a381b375ef87fb4a284f3fefe6e60c679"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.2/illumio-tui-0.5.2-x86_64-unknown-linux-musl.tar.gz"
      sha256 "1b465336440119b1d367bf545bc17fc0208a14060fc1432002bcd051e273a77e"
    end
  end

  def install
    bin.install "illumio-tui"
    # `illumio-tui completions bash|zsh|fish` prints each script.
    generate_completions_from_executable(bin/"illumio-tui", "completions")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
    assert_match "__complete profiles", shell_output("#{bin}/illumio-tui completions zsh")
  end
end
