# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.3.4"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.4/illumio-tui-0.3.4-aarch64-apple-darwin.tar.gz"
      sha256 "a058768ee210b5ece186c63589319a64d3dca7114bb056ffe0ff16191a78b05a"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.4/illumio-tui-0.3.4-x86_64-apple-darwin.tar.gz"
      sha256 "e40e2722040ee33801b65cbfaaa33363bdc851410b0408a2b448ffca9d89169f"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.4/illumio-tui-0.3.4-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "7f03f577c90c66986b6cc1c535699c96dcded6378acb0fc9740aab7cb7ec988a"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.4/illumio-tui-0.3.4-x86_64-unknown-linux-musl.tar.gz"
      sha256 "cdcae66bb12b113dad853ac4958a230af825288bc0554683f383faa0b5524bd6"
    end
  end

  def install
    bin.install "illumio-tui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
  end
end
