# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.3.2"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.2/illumio-tui-0.3.2-aarch64-apple-darwin.tar.gz"
      sha256 "cf07fad5b963ebb5ea26aac2ff819375c0f7a859e785cc4c546d4728eeed72b0"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.2/illumio-tui-0.3.2-x86_64-apple-darwin.tar.gz"
      sha256 "2709984dc0cd595f017cc5b412ab57e721f47c248ef9e3c406dde528223b434c"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.2/illumio-tui-0.3.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "ecdba5370a247a329e21b4ce9026833172f18e5b7085b1fee7c8442f396d3e3e"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.2/illumio-tui-0.3.2-x86_64-unknown-linux-musl.tar.gz"
      sha256 "8beaa30b25384291354e2e3af2c82cc3b4aa562f06c296fc0fa77e0f3233e3f3"
    end
  end

  def install
    bin.install "illumio-tui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
  end
end
