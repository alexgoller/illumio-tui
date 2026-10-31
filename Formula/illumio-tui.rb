# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.9"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.9/illumio-tui-0.5.9-aarch64-apple-darwin.tar.gz"
      sha256 "031cfd68346a3840206e5789b9fb52c5aae52d781f51d18f4759d7da31543674"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.9/illumio-tui-0.5.9-x86_64-apple-darwin.tar.gz"
      sha256 "959208e98883ec5d2d731df57e72d27bc6d4b9c2b244cf7bef9c05a4ab97c7a4"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.9/illumio-tui-0.5.9-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "04489c2a248228e700c7d75f965e787b3420f5f353babede324d4060b6c9dd75"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.9/illumio-tui-0.5.9-x86_64-unknown-linux-musl.tar.gz"
      sha256 "f4973d0452dd3db6f92788b07117ebe41f227ca4a3c89b4760a92ae93e397575"
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
