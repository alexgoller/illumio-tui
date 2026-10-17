# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.3.3"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.3/illumio-tui-0.3.3-aarch64-apple-darwin.tar.gz"
      sha256 "c68780af344553654f0e7ce60ae6b19eab076567aaf8b234d99f1440b392e4c3"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.3/illumio-tui-0.3.3-x86_64-apple-darwin.tar.gz"
      sha256 "8f0ef5016cc084767a0153cc2c8758a73f2b7cd03f8d562f32fb784ee3ba09c2"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.3/illumio-tui-0.3.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "12e497bd4489b84fba360eeb069bfc6431fcfa627fd5dc5f46c8781eae2e56ef"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.3.3/illumio-tui-0.3.3-x86_64-unknown-linux-musl.tar.gz"
      sha256 "86c68ab2cd60adcce5c2d90391e2b6713bf7821ac949b52f8b2a4bd935c5d215"
    end
  end

  def install
    bin.install "illumio-tui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
  end
end
