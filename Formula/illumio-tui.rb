# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.3.5"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.5/illumio-tui-0.3.5-aarch64-apple-darwin.tar.gz"
      sha256 "89e8438873dde5002329e31d48b1945c2b6471f5d7b8655c6b852a82411b209c"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.5/illumio-tui-0.3.5-x86_64-apple-darwin.tar.gz"
      sha256 "0baa08241052c6759225c53d483b6a654713bdeac48a471fbefb7da77d1e5a08"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.5/illumio-tui-0.3.5-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "70088c419612ee0dba39e75c315cc2ada2920d2f2e698a849bf27e6b25fd176d"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.5/illumio-tui-0.3.5-x86_64-unknown-linux-musl.tar.gz"
      sha256 "538cfb0d9ba8313f868fc9e611847d8ea6b7dc6f34967c94c91f08bfcdc092ac"
    end
  end

  def install
    bin.install "illumio-tui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
  end
end
