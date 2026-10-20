# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.4.1"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.1/illumio-tui-0.4.1-aarch64-apple-darwin.tar.gz"
      sha256 "c53d85f67ac48d8425af3fc9b1c1cf542cad93a067a1d8cf8b992c699fc283b9"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.1/illumio-tui-0.4.1-x86_64-apple-darwin.tar.gz"
      sha256 "5fe10137607a34f1841f5623287e9bc2714842b9d8f8d7255285dae7faffa2a8"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.1/illumio-tui-0.4.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "968b32b4a77315783ad20719428335a0eefc4dbb77034c16e4d51a750905ba4f"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.1/illumio-tui-0.4.1-x86_64-unknown-linux-musl.tar.gz"
      sha256 "da7f0b72e2692c38679ad78436ba64395ecfd4e0e2db32e63a509bae16fd7861"
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
