# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.3.1"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.1/illumio-tui-0.3.1-aarch64-apple-darwin.tar.gz"
      sha256 "7bfafbbe235ee729ddb8aaf819fe887bab4dfa578dadfeb1c684bdc37564298a"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.1/illumio-tui-0.3.1-x86_64-apple-darwin.tar.gz"
      sha256 "cf3a31f7e6fadae2c60523e11ae9ac832ea06daff0a5e9cc67a74607bbec62f0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.1/illumio-tui-0.3.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "f8b123546ecb640c567c47eba3882bf3ce3130702d60ff75f737463c88d893ae"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.1/illumio-tui-0.3.1-x86_64-unknown-linux-musl.tar.gz"
      sha256 "0bc6a38cc5752b30e891264ceb2b2af71134ccab775d96ead9744ef49ecb8f29"
    end
  end

  def install
    bin.install "illumio-tui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
  end
end
