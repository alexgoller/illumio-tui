# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.1"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.1/illumio-tui-0.5.1-aarch64-apple-darwin.tar.gz"
      sha256 "58687195b055dfdc62f9afbfad426617cd49bfbc9df410eee3dcb5d0d3e2f62f"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.1/illumio-tui-0.5.1-x86_64-apple-darwin.tar.gz"
      sha256 "53c08a5dd1192fcc4be9dc220e1a7f99413bcfd9ce1c67d74eb305bfc4e36564"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.1/illumio-tui-0.5.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "fac42ea5ccd2c8b519296dc8b133d4c97130a29c9b627db34e927f51a754bceb"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.1/illumio-tui-0.5.1-x86_64-unknown-linux-musl.tar.gz"
      sha256 "a5c5669c2a06aceebb8af24a0504b47752e8ee3d06742f8456664d1f00f568cc"
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
