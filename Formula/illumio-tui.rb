# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.2.0"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.2.0/illumio-tui-0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "82abe239e23e84ebbc887d8615f88132248bd5a53e48fa7a107a8c5b9744d277"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.2.0/illumio-tui-0.2.0-x86_64-apple-darwin.tar.gz"
      sha256 "d334fa57e67efe9986f30029119a575af6bfdc677d81bc53b98409f15121aa19"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.2.0/illumio-tui-0.2.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "b1642b377a80ef4d92ef98d969bd3d31c5ad7b7437ac7cb32cf7684dc907aa22"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.2.0/illumio-tui-0.2.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "c7c9c0546ce5e3aa0793dfe26d0961f0661debf895ab54c3829e0daba2c26862"
    end
  end

  def install
    bin.install "illumio-tui"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
  end
end
