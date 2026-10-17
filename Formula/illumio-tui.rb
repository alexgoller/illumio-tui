# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.3.0"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.0/illumio-tui-0.3.0-aarch64-apple-darwin.tar.gz"
      sha256 "9916bc67ce08c4734cdfc1acecf6283edba86550b25fb85033c08fb2324aaa45"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.0/illumio-tui-0.3.0-x86_64-apple-darwin.tar.gz"
      sha256 "e8cf9846ed013e71e01b2fe3656631b8064d39f857569706fa98d114ad06a5e0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.0/illumio-tui-0.3.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "34452368fa6341302df3e3b67e73f9bc743550313928ca2ae56a22887bfb005d"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.0/illumio-tui-0.3.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "241bbfbf36eb7ae8e32d0b1627d8bf86069501ff78463a3b70c058c6449a84ec"
    end
  end

  def install
    bin.install "illumio-tui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
  end
end
